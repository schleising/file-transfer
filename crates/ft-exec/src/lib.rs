//! SSH + Homebrew rsync orchestration (no filenames logged).

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Connection / path identity used by the executor (no DB types).
#[derive(Debug, Clone)]
pub struct HostRef {
    pub is_local: bool,
    pub ssh_destination: String,
    pub ssh_port: Option<u16>,
    pub identity_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub current_file: Option<String>,
    pub indeterminate: bool,
    pub message: String,
    /// Instantaneous or smoothed transfer rate (bytes/sec), when known.
    pub bytes_per_sec: Option<f64>,
    /// Estimated seconds remaining, when total and rate are known.
    pub eta_secs: Option<u64>,
    /// Rsync's own progress percent (0–100), when parsed from progress2.
    pub percent: Option<f32>,
    /// True once rsync reports `to-chk=0` (UI may show complete; process still runs to exit).
    pub data_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    LocalCopy,
    LocalToRemote,
    RemoteToLocal,
    RemotePush,
    RemotePull,
}

#[derive(Debug, Clone)]
pub struct TransferPlan {
    pub mode: TransferMode,
    pub source: HostRef,
    pub dest: HostRef,
    pub source_base: PathBuf,
    pub dest_base: PathBuf,
    /// Relative paths under source_base (no leading slash).
    pub relative_paths: Vec<String>,
    pub bytes_total: Option<u64>,
    pub file_count: u64,
}

#[derive(Debug, Clone)]
pub struct TransferResult {
    pub bytes_transferred: u64,
    pub cancelled: bool,
    pub message: String,
}

pub fn find_homebrew_rsync() -> Result<PathBuf> {
    for candidate in ["/opt/homebrew/bin/rsync", "/usr/local/bin/rsync"] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    // Last resort: whatever `which` finds, but reject ancient system copy if possible.
    let out = Command::new("which").arg("rsync").output()?;
    if out.status.success() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() && path != "/usr/bin/rsync" {
            return Ok(PathBuf::from(path));
        }
        if path == "/usr/bin/rsync" {
            bail!("only system /usr/bin/rsync found; install Homebrew rsync: brew install rsync");
        }
    }
    bail!("Homebrew rsync not found; run: brew install rsync");
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Options for non-interactive SSH (controller→host and host→host).
/// Do **not** put `-n` here: rsync `-e ssh` needs stdin for the protocol.
fn ssh_common_opts() -> &'static [&'static str] {
    &[
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=8",
        "-o",
        "StrictHostKeyChecking=accept-new",
        // Avoid known_hosts rewrites at session end (can add a noticeable pause).
        "-o",
        "UpdateHostKeys=no",
        // Avoid ControlMaster sessions that can keep stdio open after the command exits.
        "-o",
        "ControlMaster=no",
        "-o",
        "ControlPath=none",
    ]
}

/// `ssh …` argv fragment for rsync `-e` / remote peer calls (no destination).
fn peer_ssh_command(peer: &HostRef) -> String {
    let mut parts: Vec<String> = vec!["ssh".into()];
    for o in ssh_common_opts() {
        parts.push((*o).into());
    }
    if let Some(port) = peer.ssh_port {
        parts.push("-p".into());
        parts.push(port.to_string());
    }
    if let Some(id) = &peer.identity_file {
        parts.push("-i".into());
        parts.push(id.display().to_string());
    }
    parts.join(" ")
}

fn explain_peer_ssh_error(from: &HostRef, to: &HostRef, stderr: &str) -> String {
    let msg = stderr.trim();
    if msg.contains("Host key verification failed") {
        format!(
            "Host key verification failed for peer '{to}' (as seen from '{from}'). \
             The app SSHs using exactly that name — it must match what works interactively. \
             On '{from}', run once: ssh {ssh_opts}{to} true \
             Then retry. (Saved name in the app must be identical.)",
            from = from.ssh_destination,
            to = to.ssh_destination,
            ssh_opts = {
                let mut s = String::new();
                if let Some(p) = to.ssh_port {
                    s.push_str(&format!("-p {p} "));
                }
                s
            }
        )
    } else {
        format!("peer SSH failed: {msg}")
    }
}

fn ssh_base(host: &HostRef) -> Command {
    let mut cmd = Command::new("ssh");
    // Remote one-shot commands never need local stdin; closing it avoids rare end hangs
    // when the GUI inherits a TTY (e.g. cargo run).
    cmd.arg("-n");
    for o in ssh_common_opts() {
        cmd.arg(o);
    }
    if let Some(port) = host.ssh_port {
        cmd.arg("-p").arg(port.to_string());
    }
    if let Some(id) = &host.identity_file {
        cmd.arg("-i").arg(id);
    }
    cmd.arg(&host.ssh_destination);
    cmd
}

/// SSH session that runs a remote rsync and streams progress back to the controller.
fn ssh_orchestrate(host: &HostRef) -> Command {
    let mut cmd = ssh_base(host);
    // Force a pseudo-tty so remote rsync progress is line-flushed through SSH instead of
    // block-buffered until session teardown.
    cmd.arg("-tt");
    cmd
}

/// Test non-interactive SSH to a host.
pub fn test_ssh(host: &HostRef) -> Result<()> {
    if host.is_local {
        return Ok(());
    }
    let mut cmd = ssh_base(host);
    cmd.arg("true");
    let out = cmd.output().context("spawn ssh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("SSH failed: {}", err.trim());
    }
    Ok(())
}

/// From `from`, try `ssh to true` (peer reachability). Prefer push when both work.
pub fn test_peer_ssh(from: &HostRef, to: &HostRef) -> Result<()> {
    if from.is_local || to.is_local {
        bail!("peer probe only for two remotes");
    }
    // Run peer ssh under bash so zsh doesn't interfere; use same options as controller SSH.
    let peer_cmd = format!(
        "{} {} true",
        peer_ssh_command(to),
        shell_quote(&to.ssh_destination)
    );
    let remote = format!("bash -c {}", shell_quote(&peer_cmd));
    let mut cmd = ssh_base(from);
    cmd.arg(remote);
    let out = cmd.output().context("spawn peer ssh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", explain_peer_ssh_error(from, to, &err));
    }
    Ok(())
}

pub fn list_dir(host: &HostRef, path: &Path) -> Result<Vec<DirEntry>> {
    if host.is_local {
        return list_dir_local(path);
    }
    list_dir_remote(host, path)
}

fn list_dir_local(path: &Path) -> Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for ent in std::fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))? {
        let ent = ent?;
        let meta = ent.metadata()?;
        let name = ent.file_name().to_string_lossy().to_string();
        if name == "." || name == ".." || name.starts_with('.') {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        entries.push(DirEntry {
            name,
            is_dir: meta.is_dir(),
            size: if meta.is_file() { meta.len() } else { 0 },
            mtime,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn list_dir_remote(host: &HostRef, path: &Path) -> Result<Vec<DirEntry>> {
    // Prefer python3 (identical on Linux + macOS). Avoid zsh globs and GNU vs BSD `stat -f`.
    let path_q = shell_quote(&path.to_string_lossy());
    let inner = format!(
        r#"
set +e
path={path_q}
if [ ! -d "$path" ]; then
  echo "not a directory: $path" >&2
  exit 2
fi
if command -v python3 >/dev/null 2>&1; then PY=python3
elif command -v python >/dev/null 2>&1; then PY=python
else PY=
fi
if [ -n "$PY" ]; then
  exec "$PY" -c '
import os, sys
root = sys.argv[1]
try:
    names = os.listdir(root)
except OSError as e:
    sys.stderr.write(str(e) + "\n")
    sys.exit(2)
for name in sorted(names, key=lambda s: (not os.path.isdir(os.path.join(root, s)), s.lower())):
    if name.startswith("."):
        continue
    p = os.path.join(root, name)
    try:
        st = os.stat(p)
    except OSError:
        continue
    is_dir = os.path.isdir(p)
    t = "d" if is_dir else "f"
    size = 0 if is_dir else int(st.st_size)
    mt = int(st.st_mtime)
    sys.stdout.write("%s\t%s\t%s\t%s\n" % (t, size, mt, name))
' "$path"
fi
# Fallback without Python: ls + portable stat detection (GNU vs BSD)
cd "$path" || exit 2
if stat --version >/dev/null 2>&1; then
  mt_of() {{ stat -c %Y "$1" 2>/dev/null; }}
  sz_of() {{ stat -c %s "$1" 2>/dev/null; }}
else
  mt_of() {{ stat -f %m "$1" 2>/dev/null; }}
  sz_of() {{ stat -f %z "$1" 2>/dev/null; }}
fi
ls -A 2>/dev/null | while IFS= read -r f || [ -n "$f" ]; do
  [ -n "$f" ] || continue
  case "$f" in .* ) continue ;; esac
  if [ -d "$f" ]; then t=d; sz=0
  elif [ -f "$f" ]; then t=f; sz=$(sz_of "$f"); sz=${{sz:-0}}
  else continue
  fi
  mt=$(mt_of "$f"); mt=${{mt:-0}}
  printf "%s\t%s\t%s\t%s\n" "$t" "$sz" "$mt" "$f"
done
"#,
        path_q = path_q
    );
    let remote = format!("bash -c {}", shell_quote(&inner));
    let mut cmd = ssh_base(host);
    cmd.arg(remote);
    let out = cmd.output().context("ssh list")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim();
        if msg.is_empty() {
            bail!("list failed (exit {})", out.status);
        }
        bail!("list failed: {msg}");
    }
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(4, '\t');
        let t = parts.next().unwrap_or("");
        let sz: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let mt: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let name = parts.next().unwrap_or("").to_string();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        entries.push(DirEntry {
            name,
            is_dir: t == "d",
            size: sz,
            mtime: mt,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

pub fn remote_home(host: &HostRef) -> Result<PathBuf> {
    if host.is_local {
        return dirs::home_dir().context("no home dir");
    }
    let mut cmd = ssh_base(host);
    cmd.arg("printf %s \"$HOME\"");
    let out = cmd.output().context("ssh home")?;
    if !out.status.success() {
        bail!("could not resolve remote home");
    }
    let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if home.is_empty() {
        bail!("empty remote home");
    }
    Ok(PathBuf::from(home))
}

pub fn path_exists(host: &HostRef, path: &Path) -> Result<bool> {
    if host.is_local {
        return Ok(path.is_dir() || path.is_file());
    }
    let q = shell_quote(&path.to_string_lossy());
    let mut cmd = ssh_base(host);
    cmd.arg(format!("test -e {q}"));
    Ok(cmd.status()?.success())
}

fn path_is_dir(host: &HostRef, path: &Path) -> Result<bool> {
    if host.is_local {
        return Ok(path.is_dir());
    }
    let q = shell_quote(&path.to_string_lossy());
    let mut cmd = ssh_base(host);
    cmd.arg(format!("test -d {q}"));
    Ok(cmd.status()?.success())
}

/// Expand selected entries so directories become their contained file paths.
/// `--files-from` alone often creates empty dirs unless every file is listed (or -r is
/// honored consistently); expanding is reliable across local/remote rsync.
pub fn expand_selection(host: &HostRef, base: &Path, relatives: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for rel in relatives {
        let rel = rel.trim_end_matches('/').to_string();
        if rel.is_empty() || rel == "." {
            continue;
        }
        let full = base.join(&rel);
        if path_is_dir(host, &full)? {
            let mut files = list_files_under(host, base, &rel)?;
            if files.is_empty() {
                // Keep empty directory in the transfer set.
                out.push(rel);
            } else {
                files.sort();
                out.append(&mut files);
            }
        } else {
            out.push(rel);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn list_files_under(host: &HostRef, base: &Path, rel_dir: &str) -> Result<Vec<String>> {
    if host.is_local {
        let root = base.join(rel_dir);
        let mut out = Vec::new();
        for file in walkdir_files(&root)? {
            let rel = file
                .strip_prefix(base)
                .with_context(|| format!("strip {}", base.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
        return Ok(out);
    }
    list_files_under_remote(host, base, rel_dir)
}

fn list_files_under_remote(host: &HostRef, base: &Path, rel_dir: &str) -> Result<Vec<String>> {
    let base_q = shell_quote(&base.to_string_lossy());
    let rel_q = shell_quote(rel_dir);
    let inner = format!(
        r#"
set +e
base={base_q}
rel={rel_q}
root="$base/$rel"
if [ ! -d "$root" ]; then
  echo "not a directory: $root" >&2
  exit 2
fi
if command -v python3 >/dev/null 2>&1; then PY=python3
elif command -v python >/dev/null 2>&1; then PY=python
else PY=
fi
if [ -n "$PY" ]; then
  exec "$PY" -c '
import os, sys
base, rel = sys.argv[1], sys.argv[2]
root = os.path.join(base, rel)
for dirpath, dirnames, filenames in os.walk(root):
    dirnames[:] = [d for d in dirnames if not d.startswith(".")]
    for name in filenames:
        if name.startswith("."):
            continue
        full = os.path.join(dirpath, name)
        print(os.path.relpath(full, base).replace(os.sep, "/"))
' "$base" "$rel"
fi
# find fallback
find "$root" -type f ! -path "*/.*" -print 2>/dev/null | while IFS= read -r f; do
  printf "%s\n" "${{f#"$base"/}}"
done
"#,
        base_q = base_q,
        rel_q = rel_q
    );
    let remote = format!("bash -c {}", shell_quote(&inner));
    let mut cmd = ssh_base(host);
    cmd.arg(remote);
    let out = cmd.output().context("ssh list files under dir")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("expand folder failed: {}", err.trim());
    }
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let p = line.trim();
        if p.is_empty() || p.starts_with('.') || p.contains("/.") {
            continue;
        }
        files.push(p.to_string());
    }
    Ok(files)
}

/// Sum sizes for selected relative paths under base.
pub fn preflight_size(host: &HostRef, base: &Path, relatives: &[String]) -> Result<(u64, u64)> {
    if relatives.is_empty() {
        return Ok((0, 0));
    }
    if host.is_local {
        return preflight_size_local(base, relatives);
    }
    preflight_size_remote(host, base, relatives)
}

fn preflight_size_local(base: &Path, relatives: &[String]) -> Result<(u64, u64)> {
    let mut bytes = 0u64;
    let mut files = 0u64;
    for rel in relatives {
        let p = base.join(rel);
        if p.is_file() {
            bytes += p.metadata()?.len();
            files += 1;
        } else if p.is_dir() {
            for ent in walkdir_files(&p)? {
                bytes += ent.metadata()?.len();
                files += 1;
            }
        }
    }
    Ok((bytes, files))
}

fn walkdir_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for ent in std::fs::read_dir(dir)? {
            let ent = ent?;
            let p = ent.path();
            if p.is_dir() {
                walk(&p, out)?;
            } else if p.is_file() {
                out.push(p);
            }
        }
        Ok(())
    }
    walk(root, &mut out)?;
    Ok(out)
}

fn preflight_size_remote(host: &HostRef, base: &Path, relatives: &[String]) -> Result<(u64, u64)> {
    let base_q = shell_quote(&base.to_string_lossy());
    let mut list = String::new();
    for rel in relatives {
        list.push_str(&shell_quote(rel));
        list.push(' ');
    }
    let script = format!(
        r#"base={base_q}
bytes=0
files=0
for rel in {list}; do
  p="$base/$rel"
  if [ -f "$p" ]; then
    s=$(wc -c <"$p" | tr -d ' ')
    bytes=$((bytes+s)); files=$((files+1))
  elif [ -d "$p" ]; then
    while IFS= read -r f; do
      s=$(wc -c <"$f" | tr -d ' ')
      bytes=$((bytes+s)); files=$((files+1))
    done <<EOF
$(find "$p" -type f 2>/dev/null)
EOF
  fi
done
printf '%s %s\n' "$bytes" "$files"
"#,
        base_q = base_q,
        list = list
    );
    let mut cmd = ssh_base(host);
    cmd.arg(script);
    let out = cmd.output().context("size preflight")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("size preflight failed: {}", err.trim());
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let mut parts = line.split_whitespace();
    let bytes: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let files: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Ok((bytes, files))
}

pub fn plan_transfer(
    source: HostRef,
    dest: HostRef,
    source_base: PathBuf,
    dest_base: PathBuf,
    relative_paths: Vec<String>,
) -> Result<TransferPlan> {
    let relative_paths =
        expand_selection(&source, &source_base, &relative_paths).unwrap_or(relative_paths);

    let (bytes_total, file_count) = match preflight_size(&source, &source_base, &relative_paths) {
        Ok(v) => (Some(v.0), v.1),
        Err(_) => (None, relative_paths.len() as u64),
    };

    let mode = match (source.is_local, dest.is_local) {
        (true, true) => TransferMode::LocalCopy,
        (true, false) => TransferMode::LocalToRemote,
        (false, true) => TransferMode::RemoteToLocal,
        (false, false) => {
            // Prefer push; fall back to pull.
            match test_peer_ssh(&source, &dest) {
                Ok(()) => TransferMode::RemotePush,
                Err(push_err) => match test_peer_ssh(&dest, &source) {
                    Ok(()) => TransferMode::RemotePull,
                    Err(pull_err) => bail!(
                        "cannot reach peer either way. Push: {push_err:#}. Pull: {pull_err:#}"
                    ),
                },
            }
        }
    };

    Ok(TransferPlan {
        mode,
        source,
        dest,
        source_base,
        dest_base,
        relative_paths,
        bytes_total,
        file_count,
    })
}

/// Preflight that Start can proceed (controller access as needed).
pub fn preflight_start(plan: &TransferPlan) -> Result<()> {
    find_homebrew_rsync()?;
    if !plan.source.is_local {
        test_ssh(&plan.source).context("cannot SSH to source")?;
    }
    if !plan.dest.is_local {
        // Dest must be reachable from controller for validation / some modes;
        // for remote-remote push, controller still benefits from knowing dest exists.
        let _ = test_ssh(&plan.dest);
    }
    match plan.mode {
        TransferMode::RemotePush => {
            test_peer_ssh(&plan.source, &plan.dest)
                .context("source cannot SSH to destination (needed for push)")?;
        }
        TransferMode::RemotePull => {
            test_peer_ssh(&plan.dest, &plan.source)
                .context("destination cannot SSH to source (needed for pull)")?;
        }
        _ => {}
    }
    if plan.relative_paths.is_empty() {
        bail!("no files selected");
    }
    Ok(())
}

pub fn run_transfer(
    plan: &TransferPlan,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(Progress),
) -> Result<TransferResult> {
    preflight_start(plan)?;
    let rsync = find_homebrew_rsync()?;
    // Re-expand in case plan was built without expansion; ensures folder contents transfer.
    let paths = expand_selection(&plan.source, &plan.source_base, &plan.relative_paths)
        .unwrap_or_else(|_| plan.relative_paths.clone());
    if paths.is_empty() {
        bail!("nothing to transfer after expanding folders");
    }
    let files_from = write_files_from_list(&paths)?;

    let mut plan = plan.clone();
    plan.relative_paths = paths;
    let plan = &plan;

    let result = match plan.mode {
        TransferMode::LocalCopy | TransferMode::LocalToRemote | TransferMode::RemoteToLocal => {
            run_rsync_local_client(plan, &rsync, &files_from, cancel.clone(), &on_progress)
        }
        TransferMode::RemotePush => run_remote_orchestrated(
            plan,
            &rsync,
            &files_from,
            true,
            cancel.clone(),
            &on_progress,
        ),
        TransferMode::RemotePull => run_remote_orchestrated(
            plan,
            &rsync,
            &files_from,
            false,
            cancel.clone(),
            &on_progress,
        ),
    };

    let _ = std::fs::remove_file(&files_from);
    result
}

fn write_files_from_list(relatives: &[String]) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("ft-files-{}.txt", uuid::Uuid::new_v4()));
    let mut f = std::fs::File::create(&path)?;
    for rel in relatives {
        writeln!(f, "{rel}")?;
    }
    f.flush()?;
    f.sync_all()?;
    Ok(path)
}

fn rsync_common_args(cmd: &mut Command) {
    cmd.arg("-a")
        .arg("-r") // explicit: with --files-from, ensure directories are scanned
        .arg("--inplace") // write destination in place (no .<name>.XXXXXX temp + rename)
        .arg("--info=progress2")
        // Not a TTY when piped: without this, progress/stderr can be block-buffered and
        // flush only when the process exits (looks like a long hang at ~100%).
        .arg("--outbuf=N");
}

fn run_rsync_local_client(
    plan: &TransferPlan,
    rsync: &Path,
    files_from: &Path,
    cancel: Arc<AtomicBool>,
    on_progress: &impl Fn(Progress),
) -> Result<TransferResult> {
    let mut cmd = Command::new(rsync);
    rsync_common_args(&mut cmd);
    cmd.arg("--files-from").arg(files_from);

    // Trailing slash semantics: base paths as roots for relative files-from.
    let src = match plan.mode {
        TransferMode::RemoteToLocal => {
            format!(
                "{}:{}/",
                plan.source.ssh_destination,
                plan.source_base.to_string_lossy().trim_end_matches('/')
            )
        }
        _ => format!(
            "{}/",
            plan.source_base.to_string_lossy().trim_end_matches('/')
        ),
    };

    let dst = match plan.mode {
        TransferMode::LocalToRemote => {
            cmd.arg("-e").arg(ssh_rsh_args(&plan.dest));
            maybe_rsync_path_for_remote(&mut cmd, &plan.dest);
            format!(
                "{}:{}/",
                plan.dest.ssh_destination,
                plan.dest_base.to_string_lossy().trim_end_matches('/')
            )
        }
        TransferMode::RemoteToLocal => {
            cmd.arg("-e").arg(ssh_rsh_args(&plan.source));
            maybe_rsync_path_for_remote(&mut cmd, &plan.source);
            format!(
                "{}/",
                plan.dest_base.to_string_lossy().trim_end_matches('/')
            )
        }
        TransferMode::LocalCopy => {
            format!(
                "{}/",
                plan.dest_base.to_string_lossy().trim_end_matches('/')
            )
        }
        _ => unreachable!(),
    };

    cmd.arg(&src).arg(&dst);
    // progress2 is written to stdout (rsync 3.x); must drain it or the process can block.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn().context("spawn rsync")?;
    watch_child(child, plan.bytes_total, cancel, on_progress)
}

fn ssh_rsh_args(host: &HostRef) -> String {
    peer_ssh_command(host)
}

fn maybe_rsync_path_for_remote(cmd: &mut Command, _host: &HostRef) {
    // Prefer Homebrew rsync on macOS remotes; fall back to PATH `rsync` on Linux.
    cmd.arg("--rsync-path=rsync");
}

fn run_remote_orchestrated(
    plan: &TransferPlan,
    _local_rsync: &Path,
    files_from: &Path,
    push: bool,
    cancel: Arc<AtomicBool>,
    on_progress: &impl Fn(Progress),
) -> Result<TransferResult> {
    let runner = if push { &plan.source } else { &plan.dest };
    let peer = if push { &plan.dest } else { &plan.source };

    // Upload files-from list to runner temp.
    let remote_list = format!("/tmp/ft-files-{}.txt", uuid::Uuid::new_v4());
    {
        let mut cmd = Command::new("scp");
        for o in ssh_common_opts() {
            cmd.arg(o);
        }
        if let Some(port) = runner.ssh_port {
            cmd.arg("-P").arg(port.to_string());
        }
        if let Some(id) = &runner.identity_file {
            cmd.arg("-i").arg(id);
        }
        cmd.arg(files_from)
            .arg(format!("{}:{}", runner.ssh_destination, remote_list));
        let out = cmd.output().context("scp files-from")?;
        if !out.status.success() {
            bail!(
                "failed to upload file list: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    let src_base = plan
        .source_base
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    let dst_base = plan
        .dest_base
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    let rsync_remote = "rsync";
    let ssh_e = peer_ssh_command(peer);

    let script = format!(
        "RSYNC=$(command -v rsync); \
         [ -x /usr/local/bin/rsync ] && RSYNC=/usr/local/bin/rsync; \
         [ -x /opt/homebrew/bin/rsync ] && RSYNC=/opt/homebrew/bin/rsync; \
         if [ -z \"$RSYNC\" ]; then echo 'rsync missing on this host' >&2; exit 127; fi; \
         {body}",
        body = if push {
            format!(
                "if command -v stdbuf >/dev/null 2>&1; then \
                   stdbuf -oL \"$RSYNC\" -a -r --inplace --info=progress2 --outbuf=N --files-from={list} -e {ssh_e} \
                   --rsync-path={rpath} {src}/ {peer}:{dst}/; \
                 else \
                   \"$RSYNC\" -a -r --inplace --info=progress2 --outbuf=N --files-from={list} -e {ssh_e} \
                   --rsync-path={rpath} {src}/ {peer}:{dst}/; \
                 fi; ec=$?; rm -f {list}; exit $ec",
                list = shell_quote(&remote_list),
                ssh_e = shell_quote(&ssh_e),
                rpath = shell_quote(rsync_remote),
                src = shell_quote(&src_base),
                peer = shell_quote(&peer.ssh_destination),
                dst = shell_quote(&dst_base),
            )
        } else {
            format!(
                "if command -v stdbuf >/dev/null 2>&1; then \
                   stdbuf -oL \"$RSYNC\" -a -r --inplace --info=progress2 --outbuf=N --files-from={list} -e {ssh_e} \
                   --rsync-path={rpath} {peer}:{src}/ {dst}/; \
                 else \
                   \"$RSYNC\" -a -r --inplace --info=progress2 --outbuf=N --files-from={list} -e {ssh_e} \
                   --rsync-path={rpath} {peer}:{src}/ {dst}/; \
                 fi; ec=$?; rm -f {list}; exit $ec",
                list = shell_quote(&remote_list),
                ssh_e = shell_quote(&ssh_e),
                rpath = shell_quote(rsync_remote),
                src = shell_quote(&src_base),
                peer = shell_quote(&peer.ssh_destination),
                dst = shell_quote(&dst_base),
            )
        }
    );

    let mut cmd = ssh_orchestrate(runner);
    cmd.arg(script);
    // Remote progress2 arrives on ssh stdout; drain both pipes or ssh can stall on a full buffer.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().context("spawn remote rsync via ssh")?;
    watch_child(child, plan.bytes_total, cancel, on_progress)
}

fn watch_child(
    mut child: Child,
    bytes_total: Option<u64>,
    cancel: Arc<AtomicBool>,
    on_progress: &impl Fn(Progress),
) -> Result<TransferResult> {
    // rsync --info=progress2 writes to stdout (\r-separated). Drain stdout+stderr or
    // the child (especially ssh) can block once the pipe buffer fills.
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;

    let cancel_watcher = cancel.clone();
    let pid_hint = child.id();
    std::thread::spawn(move || {
        while !cancel_watcher.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = Command::new("kill").arg(pid_hint.to_string()).output();
    });

    let (tx, rx) = mpsc::channel::<RsyncChunk>();
    {
        let tx = tx.clone();
        std::thread::spawn(move || drain_rsync_stream(stdout, StreamKind::Stdout, tx));
    }
    {
        let tx = tx;
        std::thread::spawn(move || drain_rsync_stream(stderr, StreamKind::Stderr, tx));
    }

    let mut bytes_done = 0u64;
    let mut err_buf = String::new();
    let mut rate_bps: Option<f64> = None;
    let started = Instant::now();
    let mut last_sample = started;
    let mut last_bytes = 0u64;
    let mut child_status: Option<ExitStatus> = None;
    let mut streams_done = 0u8;
    // After the process exits, do not wait indefinitely for pipe EOF — a grandchild
    // (ssh) can keep a write end open briefly after the parent has exited.
    let mut exited_at: Option<Instant> = None;
    let mut data_complete = false;

    loop {
        if cancel.load(Ordering::SeqCst) {
            break;
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(RsyncChunk::Done) => {
                streams_done = streams_done.saturating_add(1);
            }
            Ok(chunk) => {
                let (line, is_err_text) = match chunk {
                    RsyncChunk::ProgressLine(line) => (line, false),
                    RsyncChunk::ErrText(line) => (line, true),
                    RsyncChunk::Done => unreachable!(),
                };
                if let Some(parsed) = parse_progress2(&line) {
                    bytes_done = parsed.bytes_done;
                    if let Some(r) = parsed.bytes_per_sec {
                        rate_bps = Some(r);
                    } else {
                        let dt = last_sample.elapsed().as_secs_f64();
                        if dt >= 0.5 && bytes_done >= last_bytes {
                            let instant = (bytes_done - last_bytes) as f64 / dt;
                            rate_bps = Some(match rate_bps {
                                Some(prev) => prev * 0.7 + instant * 0.3,
                                None => instant,
                            });
                            last_sample = Instant::now();
                            last_bytes = bytes_done;
                        } else if rate_bps.is_none() {
                            let elapsed = started.elapsed().as_secs_f64().max(0.001);
                            rate_bps = Some(bytes_done as f64 / elapsed);
                        }
                    }
                    if transfer_data_complete(&parsed) {
                        data_complete = true;
                    }
                    let mut prog = make_progress(
                        bytes_done,
                        bytes_total,
                        rate_bps,
                        parsed.percent.map(|p| p as f32),
                        data_complete,
                    );
                    if data_complete {
                        prog.message = "Done".into();
                        prog.eta_secs = Some(0);
                    }
                    on_progress(prog);
                    // Keep waiting for rsync to exit. Progress can hit 100% / to-chk=0
                    // while the last file is still being written; killing here truncated it.
                } else if is_err_text && !line.trim().is_empty() {
                    err_buf.push_str(&line);
                    if !line.ends_with('\n') {
                        err_buf.push('\n');
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                streams_done = 2;
            }
        }

        if child_status.is_none() {
            child_status = child.try_wait().context("try_wait")?;
        }
        if child_status.is_some() && exited_at.is_none() {
            exited_at = Some(Instant::now());
        }

        let pipes_settled = streams_done >= 2
            || exited_at.is_some_and(|t| t.elapsed() > Duration::from_millis(250));
        if child_status.is_some() && pipes_settled {
            break;
        }
    }

    // Drain any trailing progress/error lines briefly after exit.
    if streams_done < 2 {
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(RsyncChunk::Done) => {
                    streams_done = streams_done.saturating_add(1);
                    if streams_done >= 2 {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(RsyncChunk::ErrText(line)) => {
                    err_buf.push_str(&line);
                    if !line.ends_with('\n') {
                        err_buf.push('\n');
                    }
                }
                Ok(RsyncChunk::ProgressLine(line)) => {
                    if let Some(parsed) = parse_progress2(&line) {
                        bytes_done = parsed.bytes_done;
                        if let Some(r) = parsed.bytes_per_sec {
                            rate_bps = Some(r);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    let status = match child_status {
        Some(s) => s,
        None => child.wait().context("wait")?,
    };
    let cancelled = cancel.load(Ordering::SeqCst);

    if cancelled {
        return Ok(TransferResult {
            bytes_transferred: bytes_done,
            cancelled: true,
            message: "Cancelled".into(),
        });
    }
    if !status.success() {
        let summary = sanitize_error(&err_buf);
        bail!("transfer failed: {summary}");
    }
    if let Some(total) = bytes_total {
        bytes_done = bytes_done.max(total);
    }
    on_progress(Progress {
        bytes_done,
        bytes_total,
        current_file: None,
        indeterminate: false,
        message: "Done".into(),
        bytes_per_sec: rate_bps,
        eta_secs: Some(0),
        percent: Some(100.0),
        data_complete: true,
    });
    Ok(TransferResult {
        bytes_transferred: bytes_done,
        cancelled: false,
        message: "OK".into(),
    })
}

fn transfer_data_complete(parsed: &ParsedProgress) -> bool {
    // `100%` alone is not enough: progress2 can report 100% while ir-chk still
    // has files left (including the last file in a folder).
    parsed.to_chk.is_some_and(|(left, _)| left == 0)
}

fn make_progress(
    bytes_done: u64,
    bytes_total: Option<u64>,
    rate_bps: Option<f64>,
    percent: Option<f32>,
    data_complete: bool,
) -> Progress {
    let eta_secs = if data_complete {
        Some(0)
    } else {
        match (bytes_total, rate_bps) {
            (Some(total), Some(rate)) if rate > 1.0 && total > bytes_done => {
                Some(((total - bytes_done) as f64 / rate).ceil() as u64)
            }
            (Some(total), _) if total <= bytes_done => Some(0),
            _ => None,
        }
    };
    Progress {
        bytes_done,
        bytes_total,
        current_file: None,
        indeterminate: bytes_total.is_none() && percent.is_none(),
        message: String::new(),
        bytes_per_sec: rate_bps,
        eta_secs,
        percent,
        data_complete,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Stdout,
    Stderr,
}

enum RsyncChunk {
    ProgressLine(String),
    ErrText(String),
    Done,
}

fn drain_rsync_stream(mut stream: impl Read, kind: StreamKind, tx: mpsc::Sender<RsyncChunk>) {
    let mut buf = [0u8; 4096];
    let mut acc = Vec::new();
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\r' || b == b'\n' {
                        if !acc.is_empty() {
                            let line = String::from_utf8_lossy(&acc).into_owned();
                            acc.clear();
                            let _ = emit_rsync_line(kind, line, &tx);
                        }
                    } else {
                        acc.push(b);
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !acc.is_empty() {
        let line = String::from_utf8_lossy(&acc).into_owned();
        let _ = emit_rsync_line(kind, line, &tx);
    }
    let _ = tx.send(RsyncChunk::Done);
}

fn emit_rsync_line(
    kind: StreamKind,
    line: String,
    tx: &mpsc::Sender<RsyncChunk>,
) -> Result<(), mpsc::SendError<RsyncChunk>> {
    match kind {
        // progress2 lands on stdout for modern rsync; still accept it on stderr.
        StreamKind::Stdout | StreamKind::Stderr if parse_progress2(&line).is_some() => {
            tx.send(RsyncChunk::ProgressLine(line))
        }
        StreamKind::Stderr => tx.send(RsyncChunk::ErrText(line)),
        StreamKind::Stdout => Ok(()), // ignore non-progress stdout
    }
}

struct ParsedProgress {
    bytes_done: u64,
    bytes_per_sec: Option<f64>,
    percent: Option<u32>,
    /// Remaining / total from `(xfr#…, to-chk=A/B)` or `ir-chk=A/B`.
    to_chk: Option<(u64, u64)>,
}

/// Parse rsync `--info=progress2` lines like:
/// `  1,048,576  50%  10.00MB/s    0:00:01 (xfr#1, to-chk=3/10)`
/// Note: modern rsync writes these to **stdout** when not a TTY.
fn parse_progress2(line: &str) -> Option<ParsedProgress> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if !t.contains('%') && !t.contains("xfr#") {
        return None;
    }
    let mut parts = t.split_whitespace();
    let first = parts.next()?;
    let digits: String = first.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let bytes_done = digits.parse().ok()?;
    let mut bytes_per_sec = None;
    let mut percent = None;
    for tok in parts {
        if percent.is_none() {
            if let Some(p) = tok.strip_suffix('%') {
                if let Ok(v) = p.parse::<u32>() {
                    percent = Some(v);
                    continue;
                }
            }
        }
        if bytes_per_sec.is_none() {
            if let Some(rate) = parse_rate_token(tok) {
                bytes_per_sec = Some(rate);
            }
        }
    }
    let to_chk = parse_chk_token(t);
    Some(ParsedProgress {
        bytes_done,
        bytes_per_sec,
        percent,
        to_chk,
    })
}

fn parse_chk_token(line: &str) -> Option<(u64, u64)> {
    for key in ["to-chk=", "ir-chk="] {
        if let Some(pos) = line.find(key) {
            let rest = &line[pos + key.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '/')
                .unwrap_or(rest.len());
            let pair = &rest[..end];
            let (a, b) = pair.split_once('/')?;
            return Some((a.parse().ok()?, b.parse().ok()?));
        }
    }
    None
}

fn parse_rate_token(tok: &str) -> Option<f64> {
    let lower = tok.to_ascii_lowercase();
    let (num, mult) = if let Some(rest) = lower.strip_suffix("gb/s") {
        (rest, 1024.0 * 1024.0 * 1024.0)
    } else if let Some(rest) = lower.strip_suffix("mb/s") {
        (rest, 1024.0 * 1024.0)
    } else if let Some(rest) = lower.strip_suffix("kb/s") {
        (rest, 1024.0)
    } else if let Some(rest) = lower.strip_suffix("b/s") {
        (rest, 1.0)
    } else {
        return None;
    };
    let n: f64 = num.parse().ok()?;
    Some(n * mult)
}

/// Back-compat helper for tests / simple callers.
pub fn parse_progress2_line(line: &str) -> Option<u64> {
    parse_progress2(line).map(|p| p.bytes_done)
}

fn sanitize_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(stderr);
    let mut s = line.trim().to_string();
    if s.len() > 200 {
        s.truncate(200);
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_progress() {
        let p =
            parse_progress2("  1,048,576  50%  10.00MB/s    0:00:01 (xfr#1, to-chk=0/1)").unwrap();
        assert_eq!(p.bytes_done, 1_048_576);
        assert!((p.bytes_per_sec.unwrap() - 10.0 * 1024.0 * 1024.0).abs() < 1.0);
        assert_eq!(p.percent, Some(50));
        assert_eq!(p.to_chk, Some((0, 1)));
    }

    #[test]
    fn parse_progress_cr_style() {
        let n = parse_progress2_line("    12345  10%  1.00MB/s    0:00:01");
        assert_eq!(n, Some(12345));
    }

    #[test]
    fn parse_ir_chk() {
        let p = parse_progress2("  999 100%  1.00MB/s    0:00:01 (xfr#2, ir-chk=12/40)").unwrap();
        assert_eq!(p.percent, Some(100));
        assert_eq!(p.to_chk, Some((12, 40)));
        assert!(!transfer_data_complete(&p));
    }

    #[test]
    fn data_complete_when_to_chk_zero() {
        let p =
            parse_progress2("  1,048,576  100%  10.00MB/s    0:00:01 (xfr#4, to-chk=0/4)").unwrap();
        assert!(transfer_data_complete(&p));
    }

    #[test]
    fn quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
