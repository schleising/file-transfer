//! SSH + Homebrew rsync orchestration (no filenames logged).

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
    for candidate in [
        "/opt/homebrew/bin/rsync",
        "/usr/local/bin/rsync",
    ] {
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
            bail!(
                "only system /usr/bin/rsync found; install Homebrew rsync: brew install rsync"
            );
        }
    }
    bail!("Homebrew rsync not found; run: brew install rsync");
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn ssh_base(host: &HostRef) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes")
        .arg("-o").arg("ConnectTimeout=8")
        .arg("-o").arg("StrictHostKeyChecking=accept-new");
    if let Some(port) = host.ssh_port {
        cmd.arg("-p").arg(port.to_string());
    }
    if let Some(id) = &host.identity_file {
        cmd.arg("-i").arg(id);
    }
    cmd.arg(&host.ssh_destination);
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
    let remote = format!(
        "ssh -o BatchMode=yes -o ConnectTimeout=8 {} true",
        shell_quote(&to.ssh_destination)
    );
    let mut cmd = ssh_base(from);
    cmd.arg(remote);
    let out = cmd.output().context("spawn peer ssh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("peer SSH failed: {}", err.trim());
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
    for ent in std::fs::read_dir(path)
        .with_context(|| format!("read_dir {}", path.display()))?
    {
        let ent = ent?;
        let meta = ent.metadata()?;
        let name = ent.file_name().to_string_lossy().to_string();
        if name == "." || name == ".." {
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
    // Portable: one line per entry — type|size|mtime|name  (name may contain | rarely; use \t sep)
    let path_q = shell_quote(&path.to_string_lossy());
    let script = format!(
        r#"cd {path_q} || exit 2
for f in * .[!.]* ..?*; do
  [ -e "$f" ] || continue
  [ "$f" = "." ] && continue
  [ "$f" = ".." ] && continue
  if [ -d "$f" ]; then t=d; else t=f; fi
  sz=$(wc -c <"$f" 2>/dev/null | tr -d ' ' || echo 0)
  mt=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || echo 0)
  printf '%s\t%s\t%s\t%s\n' "$t" "$sz" "$mt" "$f"
done"#,
        path_q = path_q
    );
    let mut cmd = ssh_base(host);
    cmd.arg(script);
    let out = cmd.output().context("ssh list")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("list failed: {}", err.trim());
    }
    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(4, '\t');
        let t = parts.next().unwrap_or("");
        let sz: u64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let mt: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let name = parts.next().unwrap_or("").to_string();
        if name.is_empty() {
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

pub fn path_exists(host: &HostRef, path: &Path) -> Result<bool> {
    if host.is_local {
        return Ok(path.is_dir() || path.is_file());
    }
    let q = shell_quote(&path.to_string_lossy());
    let mut cmd = ssh_base(host);
    cmd.arg(format!("test -e {q}"));
    Ok(cmd.status()?.success())
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
    let (bytes_total, file_count) =
        match preflight_size(&source, &source_base, &relative_paths) {
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
    let files_from = write_files_from_list(&plan.relative_paths)?;

    let result = match plan.mode {
        TransferMode::LocalCopy | TransferMode::LocalToRemote | TransferMode::RemoteToLocal => {
            run_rsync_local_client(plan, &rsync, &files_from, cancel.clone(), &on_progress)
        }
        TransferMode::RemotePush => {
            run_remote_orchestrated(plan, &rsync, &files_from, true, cancel.clone(), &on_progress)
        }
        TransferMode::RemotePull => {
            run_remote_orchestrated(plan, &rsync, &files_from, false, cancel.clone(), &on_progress)
        }
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
    Ok(path)
}

fn run_rsync_local_client(
    plan: &TransferPlan,
    rsync: &Path,
    files_from: &Path,
    cancel: Arc<AtomicBool>,
    on_progress: &impl Fn(Progress),
) -> Result<TransferResult> {
    let mut cmd = Command::new(rsync);
    cmd.arg("-a")
        .arg("--info=progress2")
        .arg("--out-format=%n")
        .arg("--files-from")
        .arg(files_from);

    // Trailing slash semantics: base paths as roots for relative files-from.
    let src = match plan.mode {
        TransferMode::RemoteToLocal => {
            format!(
                "{}:{}/",
                plan.source.ssh_destination,
                plan.source_base.to_string_lossy().trim_end_matches('/')
            )
        }
        _ => format!("{}/", plan.source_base.to_string_lossy().trim_end_matches('/')),
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
            format!("{}/", plan.dest_base.to_string_lossy().trim_end_matches('/'))
        }
        TransferMode::LocalCopy => {
            format!("{}/", plan.dest_base.to_string_lossy().trim_end_matches('/'))
        }
        _ => unreachable!(),
    };

    cmd.arg(&src).arg(&dst);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = cmd.spawn().context("spawn rsync")?;
    watch_child(child, plan.bytes_total, cancel, on_progress)
}

fn ssh_rsh_args(host: &HostRef) -> String {
    let mut s = String::from("ssh -o BatchMode=yes");
    if let Some(port) = host.ssh_port {
        s.push_str(&format!(" -p {port}"));
    }
    if let Some(id) = &host.identity_file {
        s.push_str(&format!(" -i {}", id.display()));
    }
    s
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
        cmd.arg("-o").arg("BatchMode=yes");
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

    let src_base = plan.source_base.to_string_lossy().trim_end_matches('/').to_string();
    let dst_base = plan.dest_base.to_string_lossy().trim_end_matches('/').to_string();
    let rsync_remote = "rsync";

    let script = format!(
        "RSYNC=$(command -v rsync); \
         [ -x /usr/local/bin/rsync ] && RSYNC=/usr/local/bin/rsync; \
         [ -x /opt/homebrew/bin/rsync ] && RSYNC=/opt/homebrew/bin/rsync; \
         if [ -z \"$RSYNC\" ]; then echo 'rsync missing on this host' >&2; exit 127; fi; \
         {body}",
        body = if push {
            format!(
                "\"$RSYNC\" -a --info=progress2 --files-from={list} -e 'ssh -o BatchMode=yes' \
                 --rsync-path={rpath} {src}/ {peer}:{dst}/; ec=$?; rm -f {list}; exit $ec",
                list = shell_quote(&remote_list),
                rpath = shell_quote(rsync_remote),
                src = shell_quote(&src_base),
                peer = shell_quote(&peer.ssh_destination),
                dst = shell_quote(&dst_base),
            )
        } else {
            format!(
                "\"$RSYNC\" -a --info=progress2 --files-from={list} -e 'ssh -o BatchMode=yes' \
                 --rsync-path={rpath} {peer}:{src}/ {dst}/; ec=$?; rm -f {list}; exit $ec",
                list = shell_quote(&remote_list),
                rpath = shell_quote(rsync_remote),
                src = shell_quote(&src_base),
                peer = shell_quote(&peer.ssh_destination),
                dst = shell_quote(&dst_base),
            )
        }
    );

    let mut cmd = ssh_base(runner);
    cmd.arg(script);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().context("spawn remote rsync via ssh")?;
    watch_child(child, plan.bytes_total, cancel, on_progress)
}

fn watch_child(
    mut child: Child,
    bytes_total: Option<u64>,
    cancel: Arc<AtomicBool>,
    on_progress: &impl Fn(Progress),
) -> Result<TransferResult> {
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;

    let cancel_watcher = cancel.clone();
    let pid_hint = child.id();
    std::thread::spawn(move || {
        while !cancel_watcher.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = Command::new("kill").arg(pid_hint.to_string()).status();
    });

    let mut bytes_done = 0u64;
    let mut last_file: Option<String> = None;

    let err_handle = std::thread::spawn(move || {
        let mut s = String::new();
        let reader = BufReader::new(stderr);
        for line in reader.lines().flatten() {
            s.push_str(&line);
            s.push('\n');
        }
        s
    });

    let reader = BufReader::new(stdout);
    for line in reader.lines().flatten() {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        if let Some(p) = parse_progress2_line(&line) {
            bytes_done = p;
            on_progress(Progress {
                bytes_done,
                bytes_total,
                current_file: last_file.clone(),
                indeterminate: bytes_total.is_none(),
                message: String::new(),
            });
        } else if !line.trim().is_empty()
            && !line.contains('%')
            && !line.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            // Likely --out-format filename (ephemeral UI only).
            last_file = Some(line);
            on_progress(Progress {
                bytes_done,
                bytes_total,
                current_file: last_file.clone(),
                indeterminate: bytes_total.is_none(),
                message: String::new(),
            });
        }
    }

    let err_buf = err_handle.join().unwrap_or_default();
    let status = child.wait().context("wait")?;
    let cancelled = cancel.load(Ordering::SeqCst);

    if cancelled {
        return Ok(TransferResult {
            bytes_transferred: bytes_done,
            cancelled: true,
            message: "Cancelled".into(),
        });
    }
    if !status.success() {
        // Strip likely path-looking segments from stderr for privacy in returned message.
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
    });
    Ok(TransferResult {
        bytes_transferred: bytes_done,
        cancelled: false,
        message: "OK".into(),
    })
}

/// Parse rsync `--info=progress2` lines like:
/// `  1,234,567  45%  12.34MB/s    0:00:01 (xfr#1, to-chk=3/10)`
pub fn parse_progress2_line(line: &str) -> Option<u64> {
    let t = line.trim_start();
    if t.is_empty() {
        return None;
    }
    let first = t.split_whitespace().next()?;
    let digits: String = first.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    // progress2 lines usually include % later
    if !line.contains('%') && !line.contains("xfr#") {
        return None;
    }
    digits.parse().ok()
}

fn sanitize_error(stderr: &str) -> String {
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(stderr);
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
        let n = parse_progress2_line(
            "  1,048,576  50%  10.00MB/s    0:00:01 (xfr#1, to-chk=0/1)",
        );
        assert_eq!(n, Some(1_048_576));
    }

    #[test]
    fn quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
