# Design: Direct File Transfer (Rust GUI)

**Status:** Implemented (personal-use **1.0.0**). This document describes the **as-built** system in this repo. App and local crate versions live in Cargo.toml (`workspace.package.version`, inherited unless a crate sets its own); bump with semver on shipped changes.

## 1. Overview

A **macOS** desktop application that orchestrates file transfers between computers using **SSH** and **rsync** (Homebrew on macOS). The app installs as **File Transfer.app** under `/Applications` (bundle id `local.file-transfer`). Linux machines are **source and/or destination only**—they never run this GUI.

File bytes move **directly from source to destination** when both are remote. They do not stream through the controller (avoid the `rsync user@A:… user@B:…` local-relay pitfall).

Hosts are discoverable via **Avahi / Bonjour** (`_ssh._tcp`) from the Add Location sheet. Missing SSH keys or peer access **only blocks that transfer**; the app always launches and remains usable.

On macOS the process stays running after the window is closed: it hides to a **menu-bar extra**. Quit from the extra or the File Transfer menu. **Open at Login** is a LaunchAgent (`~/Library/LaunchAgents/local.file-transfer.plist`) that starts the app hidden.

### Roles

| Role | Name | Responsibility |
|------|------|----------------|
| **Controller** | Computer 1 (macOS only) | GUI in `/Applications`: discover/save hosts, pick folders/files, run `ssh` / `rsync`, show progress |
| **Source** | Computer 2 (macOS or Linux) | Holds the files; SSH reachable; Avahi/Bonjour on the LAN |
| **Destination** | Computer 3 (macOS or Linux) | Receives via **rsync over SSH**; needs `sshd` + `rsync` binary, **not** an rsync daemon |

Source and/or destination may be the controller Mac itself.

```
                    ssh (list, orchestrate, progress telemetry)
         ┌──────────────► Controller Mac (GUI .app) ◄──────────────┐
         │                                                          │
         │         ssh: start rsync on source (push)                 │
         ▼                                                          │
    Source host  ══════ rsync over SSH (direct) ══════►  Dest host
                 (no rsyncd; dest runs rsync --server via sshd)
```

---

## 2. Goals and non-goals

### Goals

1. Run the GUI on one Mac and transfer files from a second computer to a third.
2. Persist known computers for quick selection.
3. Persist known **locations** (host + folder path) as tiles on Source and Destination.
4. List files in a source location for selection (multi-select; folders as units).
5. Select a single destination location.
6. Allow source or destination to be the controller Mac.
7. Direct source→destination data path (not via the controller).
8. No custom agent on remotes; **rsync over SSH** only—**no `rsyncd`**.
9. Reliable GUI progress for the overall job (as far as reasonably possible).
10. Avahi/Bonjour discovery of hosts (`_ssh._tcp`).
11. Always run as **File Transfer.app** from `/Applications` (personal use).
12. Privacy: never persist or log transferred **filenames**.

### Non-goals (v1)

- Linux or Windows as controller / initiator.
- `rsyncd` / port 873 on any host.
- Multi-destination transfers.
- Full remote shell admin UI.
- Cloud sync, versioning, or conflict resolution.
- Blocking app launch on incomplete SSH setup.
- Apple Developer signing, notarization, or third-party distribution.
- Transfer **History** UI or persisted job records.
- Native macOS folder picker (`rfd`).
- Dedicated Computers / Locations / History tabs (hosts and folders are managed from the transfer wizard).

---

## 3. Prerequisites

Personal use only: build locally and install via `./scripts/install-app.sh`. No Developer ID or notarization.

### Controller Mac

| Requirement | Notes |
|-------------|--------|
| macOS | Bonjour built in |
| Xcode Command Line Tools | For building |
| Rust (rustup) | Builds `ft-app` |
| Homebrew `rsync` | `brew install rsync` — not `/usr/bin/rsync` |
| System `ssh` | Key-based / agent / `~/.ssh/config` (non-interactive) |
| **File Transfer.app** | Installed to `/Applications` by the install script |

### Source / destination (macOS or Linux)

| Requirement | Notes |
|-------------|--------|
| `sshd` | From controller; peer-to-peer for A≠B≠C jobs |
| `rsync` binary | macOS: Homebrew preferred; Linux: distro package. Not a daemon |
| Avahi / Bonjour | For discovery (manual hosts still work) |
| SSH trust | Controller→hosts; source↔dest for direct remote→remote |
| `python3` or `python` (recommended) | Used for portable remote directory listing; `find`/`ls` fallback exists |
| `bash` | Remote listing/orchestration scripts run under `bash -c` (avoids zsh glob pitfalls) |

### Quick verification

1. Controller: Homebrew `rsync --version`; `ssh user@host true` for each remote.
2. Three-machine: from source, `ssh` to dest using the **exact** SSH name saved in the app.
3. App opens from `/Applications` even if some hosts are down.

---

## 4. Platform and tooling (as built)

### Controller

- macOS only; **Dioxus desktop 0.8** GUI (WKWebView, macOS-native styling). Stylesheet is `include_str!`’d at compile time (`crates/ft-app/assets/macos.css`); CSS changes require a rebuild.
- Install: `./scripts/install-app.sh` → `cargo build --release -p ft-app`, assemble minimal `.app` (`Info.plist` sets `NSQuitAlwaysKeepsWindows = false`), copy to `/Applications`.
- Local rsync: `/opt/homebrew/bin/rsync` then `/usr/local/bin/rsync` (never prefer system `/usr/bin/rsync` when Homebrew exists).
- Data dir: `~/Library/Application Support/File Transfer/` (SQLite).
- Window: default **1280×840** logical pixels, minimum **900×560**. Last size and position are restored from the store (`settings.window.frame`). First launch lets macOS place the window. Minimized / fullscreen frames are not saved.

### Remotes

- Transport: **rsync over SSH** only.
- Peer and controller SSH options include `BatchMode=yes`, `ConnectTimeout=8`, `StrictHostKeyChecking=accept-new`, `UpdateHostKeys=no`, `ControlMaster=no`, `ControlPath=none`. One-shot controller SSH uses `-n` (stdin closed); rsync `-e ssh` does **not** use `-n` (protocol needs stdin).
- Remote orchestration SSH adds `-tt` so rsync progress is line-flushed through the session; remote rsync uses `stdbuf -oL` when available.
- Remote Mac rsync: prefer Homebrew paths when resolving the client on that host; `--rsync-path=rsync` for the server side of a hop (PATH / package on Linux).

### Trust and startup

- App always starts. Login-item and session-restore launches stay **hidden** until the extra or Dock/Finder reopen the window.
- **Transfer** is gated on an automatic access/preflight check (SSH and peer reachability). Failures show as **Access status** in the sidebar; they do not crash the app.

---

## 5. Discovery (Avahi / Bonjour)

- Crate `ft-mdns` browses **`_ssh._tcp.local.`** via the `mdns-sd` crate.
- The **Add Location** sheet lists discoveries as **Discovered on Network**; **Add Host** saves into the store.
- **Add host manually** (display name, SSH destination, optional port) remains supported.
- There is no dedicated Computers tab and no in-app “Test SSH” action; reachability is the transfer preflight.
- No custom DNS-SD service type in v1.

---

## 6. User experience (as built)

### Layout

Sidebar + main stage + persistent footer.

- **Sidebar** — brand with app version (hover for crate versions); Source / Files / Destination steps; **Summary** of the current plan (source host/folder, selected files with hover list, destination host/folder); **Access status** (Untested / Testing / Accessible / Inaccessible); **Reset** (clears the plan and returns to Source); footer caption “Direct rsync over SSH”.
- **Main** — the active step. Source and Destination show **Add Location** in the page header. **Continue** / **Back** sit in the wizard bar.
- **Footer** — status, rate/ETA, progress bar, **Cancel** while transferring, primary **Transfer** (enabled only when Access status is Accessible).

Selections are locked while preflight is running or a transfer is in progress.

### Steps

1. **Source** — location tiles grouped by host. Click a folder tile to select it. Drag tiles to reorder within a host (persisted `sort_order`). Tile **×** deletes that saved location. **Add folder** on a host opens the in-app browser. **Add Location** opens the host/path sheet.
2. **Files** — lists the selected source folder (dotfiles hidden). Multi-select files and/or folders; Select All / Clear / Refresh. Continue requires at least one selection.
3. **Destination** — same tile UI as Source, for the destination folder.

### Adding a location

**Add Location** sheet: pick a saved host chip, optionally **Add Host** from Bonjour or **Add host manually**, then type an absolute path (**Use Path**) or **Browse…**. The same in-app folder browser is used for **This Mac** and remotes (list dirs, Up / Home / Go, Select this folder). New paths are upserted as locations (`kind` is always `either`).

There is no native `rfd` folder dialog.

### Transfer flow

1. **Source** — pick a saved folder (or add one) → **Continue**.
2. Multi-select files and/or folders → **Continue**.
3. **Destination** — pick a saved folder (or add one).
4. When source, files, and destination are all set, preflight runs automatically. **Access status** updates in the sidebar.
5. **Transfer** reuses the cached preflight plan (no second folder expand) → progress bar with rate and ETA; **Cancel** kills the local ssh/rsync child.
6. UI shows **Transfer complete** as soon as rsync reports payload done (see §8); SSH teardown continues in the background.
7. **Reset** clears source/files/destination and access state (not saved hosts or folders).

Selected names appear in the running UI (file list and Summary hover). They are session-only.

### Soft-fail access

Peer host-key or permission failures block **Transfer** with an explanation (including that the app SSH name must match interactive use). UI otherwise remains usable.

---

## 7. Transfer model (as built)

### No rsync daemon

Never `host::module` / port 873. Far side is `rsync --server` under `sshd`.

### Direct path

| Source | Dest | Client runs on |
|--------|------|----------------|
| Local | Local | Controller (Homebrew rsync) |
| Local | Remote | Controller |
| Remote | Local | Controller |
| Remote A | Remote B | **A** (push) if A→B SSH works; else **B** (pull) |

Controller must not run `rsync A:… B:…` (that relays via the Mac).

### Rsync flags (implemented)

- `-a -r` — archive + explicit recurse.
- `--inplace` — write the real destination path (no `.<name>.XXXXXX` temp then rename).
- `--info=progress2` — aggregate progress (parsed from **stdout** on Homebrew rsync 3.x when piped; `\r`-delimited updates).
- `--outbuf=N` — disable block buffering when stdout/stderr are pipes (progress would otherwise flush only at exit).
- `--files-from=` — exact selection; temp list deleted after the job; never stored in the DB.

### Folder selections

`--files-from` with only a directory name often created an **empty** directory. Before transfer, **`expand_selection`** walks each selected folder (local walk or remote Python/`find`) and replaces it with relative **file** paths under the source base (empty dirs kept as dir entries). Hidden names (leading `.`) are skipped in expansion listings consistently with the UI.

### Peer SSH

Probe and data-plane `-e ssh …` use the same non-interactive options as the controller (`accept-new`, etc.). Host key failures cite the exact peer name configured in the app.

### Path layout

Relative structure under the destination folder is preserved.

---

## 8. Progress (as built)

1. **Preflight** — size/count of the (expanded) selection when possible (`bytes_total`, `file_count` on the in-memory plan and Access status).
2. **Bar** — prefers rsync’s own **percent** from progress2 when available; otherwise `bytes_done / bytes_total`. Shows **transfer rate** and **ETA** when parsed or derivable. Indeterminate animation if total unknown.
3. **Parsing** — progress2 lines like `bytes  pct  rate  time (xfr#N, to-chk=L/T)`; extract bytes, percent, rate (`KB/s`/`MB/s`/…), and `to-chk` / `ir-chk` for completion detection.
4. **Stream handling** — drain **both stdout and stderr**; split on `\r` and `\n`. On rsync 3.x with piped I/O, progress2 goes to **stdout** (not stderr). Failing to drain either pipe can **block rsync** (historically stderr; stdout null also caused stalls). Use `--outbuf=N` on the client.
5. **Completion semantics** — the UI may show 100% when progress2 reports `to-chk=0`. The transfer is only **successful** after rsync/ssh **exits 0**. Do not SIGKILL on `100%` or a progress stall: that can truncate the last file in a folder (`--inplace`). Drain both pipes until the child exits (cancel still kills the controller-side child).
6. **Remote→remote** — orchestration SSH uses `-tt`; remote script prefers `stdbuf -oL` around rsync so progress streams instead of buffering until session exit.
7. UI repaints on background progress messages while a transfer runs.
8. Filenames are not persisted and are not shown in the progress bar.

### Cancel

Kill the controller-side child (ssh/rsync). Remote jobs stop when the SSH session drops.

---

## 9. Listing (as built)

- **Local:** `std::fs::read_dir`; skip names starting with `.`.
- **Remote:** prefer `python3`/`python` over SSH for portable metadata; fallback `ls` + GNU/BSD `stat` detection. Always force **`bash -c`** so zsh `nomatch` does not break listing.
- Listing format: `type\tsize\tmtime\tname`.
- Session-only; not written to the store.

---

## 10. Persistence and privacy (`ft-store`)

SQLite at `~/Library/Application Support/File Transfer/file-transfer.sqlite3`.

### Allowed

- Computers, locations, and small **settings** (currently the last window frame).

### Forbidden

- Transferred filenames / selection paths in DB or logs.
- Persisted job / History records.
- Persisting `--files-from` contents.

A leftover `jobs` table is dropped on migrate if present.

### Schema (as built)

```text
Computer {
  id, name, ssh_destination, ssh_port?, identity_file?,
  bonjour_name?, last_seen_at?, is_local, created_at, updated_at
}

Location {
  id, computer_id, name, path, kind: Source | Dest | Either,
  sort_order, created_at, updated_at
}

Setting {
  key, value     // e.g. window.frame = "x y width height" (logical px)
}
```

`identity_file` and location `kind` are stored for compatibility; the UI always writes `kind = either` and does not expose an identity-file picker. Hosts can be added from the location sheet; they cannot be deleted in-app.

On first open, a **This Mac** computer and a **Home** location are created automatically.

---

## 11. Architecture (as built)

```
crates/
  ft-app/     Dioxus UI, window frame, macOS extra, transfer threads, folder browser
  ft-exec/    ssh/rsync, listing, expand folders, progress parse, peer probe
  ft-store/   SQLite computers / locations / settings
  ft-mdns/    _ssh._tcp browse (mdns-sd)
scripts/install-app.sh   release build → File Transfer.app → /Applications
```

| Crate | Role |
|-------|------|
| `ft-app` | Dioxus UI; in-app folder browser (local and remote); menu-bar extra / Open at Login; window geometry; wires store + exec + mdns |
| `ft-exec` | All process orchestration and progress |
| `ft-store` | Persistence + privacy boundary |
| `ft-mdns` | Discovery snapshot for the Add Location sheet |

---

## 12. GUI details

- Toolkit: **Dioxus desktop 0.8** (WKWebView) with a macOS System Settings–style layout, SF Pro / `-apple-system` type, system accent, light and dark appearance.
- Layout: **sidebar wizard** (Source, Files, Destination) under a transparent full-size titlebar; brand shows the app version (hover lists `ft-app` / `ft-exec` / `ft-store` / `ft-mdns`); **Summary** + **Access status**; **persistent bottom progress bar** (status, bytes, rate, ETA, Cancel, Transfer).
- Icons: SF Symbol–style inline SVGs for nav, folders, files, network, status.
- Locations: Finder-like **tiles** grouped by host; live drag-reorder; Add folder / Add Location.
- Primary actions: **Continue** / **Reset** / **Transfer** use the system blue accent.
- Packaging: minimal `Info.plist` + `AppIcon.icns` + binary `Contents/MacOS/file-transfer` (not cargo-bundle).
- macOS extra: template menu-bar icon; left-click toggles the window; menu has Open at Login and Quit. Close button **hides** the window (`WindowCloseBehaviour::WindowHides`).

---

## 13. Security and safety (as built)

- Trust model = existing SSH credentials.
- Remote commands built with careful quoting (`shell_quote`); listing prefers Python argv path.
- No filename logging.
- Overwrite / free-space confirmation UI: **not** fully built in v1 (rsync default overwrite behavior applies).
- Redacted command details panel: **not** built in v1.

---

## 14. Error handling

| Failure | App launch | Transfer |
|---------|------------|----------|
| SSH / keys / host key | OK | Block or fail job; peer errors explain exact SSH name |
| Peer unreachable both ways | OK | Cannot plan remote→remote |
| Discovery empty | OK | Manual hosts |
| Size preflight fails | OK | Indeterminate progress allowed |
| Stderr pipe not drained (historical bug) | — | Fixed: blocked after first file |
| Stdout not drained / progress on stdout (rsync 3.x) | — | Fixed: pipe stdout; `--outbuf=N` |
| Long pause at 100% waiting for SSH teardown | — | UI can show 100% on `to-chk=0`; rsync is allowed to exit before success |
| Missing Homebrew rsync on controller | OK | Local rsync ops fail with install hint |
| Cancel | OK | Kill child; status cancelled |
| Close window | Stays in menu bar | — |
| Open at Login | Starts hidden | — |

---

## 15. Implementation status

| Area | Status |
|------|--------|
| Workspace crates + Dioxus 0.8 app | Done |
| Store + This Mac / Home defaults | Done |
| Local ↔ remote ↔ remote→remote (push/pull) | Done |
| Bonjour `_ssh._tcp` + save from Add Location | Done |
| In-app folder browse (local and remote) | Done |
| Location tiles, drag reorder, delete | Done |
| Automatic access/preflight; Transfer gated on Accessible | Done |
| Progress2 parse + dual-stream drain + `--inplace` + `--outbuf=N` | Done |
| Progress rate/ETA; wait for rsync exit (do not kill on 100%) | Done |
| Folder expand for `--files-from` | Done |
| Menu-bar extra, hide-on-close, Open at Login | Done |
| Window size/position restore | Done |
| Install script → `/Applications` | Done |
| History / job records | Not in v1 |
| Overwrite policy UI / df preflight / redacted command panel | Not in v1 |
| Signing / notarization | Out of scope |

---

## 16. Testing (manual / as used)

- Multi-file and folder transfers with visible progress, rate, and ETA.
- “Transfer complete” appears when payload finishes, not after SSH teardown.
- Remote Mac and Linux listing (bash + Python).
- Peer SSH with matching hostnames / `accept-new`.
- Privacy: DB has no per-file names (no job table).
- Install from `scripts/install-app.sh` and launch from `/Applications`.
- Close hides to the extra; Quit from the extra or app menu; Open at Login starts hidden.
- Window reopen restores last size and position.

---

## 17. Decisions (locked + as-built notes)

| Topic | Decision / as-built |
|-------|---------------------|
| GUI | **Dioxus desktop 0.8** (macOS-styled WKWebView) |
| Navigation | **Source → Files → Destination** wizard; no Computers / Locations / History tabs |
| Locations | **Tiles per host**; drag to reorder; Add Location sheet for hosts + paths |
| Folder pick | **In-app browser** for both This Mac and remotes (not `rfd`) |
| Access | **Automatic preflight** caches the plan; Transfer enabled only when Accessible |
| Window | Default **1280×840**; persist logical size + position in `settings` |
| Close / login | **Hide to menu-bar extra**; Open at Login via LaunchAgent `--hidden` |
| Remote→remote | Probe both; **prefer push** |
| Progress | Preflight total + parse **progress2 from stdout** (`\r`); `--outbuf=N`; drain both pipes; rate/ETA; **wait for rsync exit** |
| DNS-SD | **`_ssh._tcp` only** |
| App name / run mode | **File Transfer.app** in `/Applications` (`local.file-transfer`) |
| Distribution | Personal build; `scripts/install-app.sh` (no cargo-bundle, no notarization) |
| Rsync write mode | **`--inplace`** |
| Folder select | **Expand to file list** before `--files-from` |
| Remote list | **Python preferred**; bash wrapper; hide `.*` in UI listings |
| Peer SSH | Shared opts including **`StrictHostKeyChecking=accept-new`**, **`UpdateHostKeys=no`**, **`ControlMaster=no`** |
| Remote orchestration SSH | **`-tt`** + remote **`stdbuf -oL`** when available |

---

## 18. Success criteria (met for v1)

- File Transfer.app on the controller Mac initiates transfers; Linux never needs the app.
- A≠B≠C jobs move payload B↔C via rsync over SSH (push preferred).
- Progress bar updates during multi-file jobs without stalling rsync; shows rate and ETA.
- “Transfer complete” when rsync payload is done, not when SSH finishes closing.
- Host/folder locations persist as tiles; filenames never stored.
- Broken SSH setup does not prevent startup.
- Controller uses Homebrew rsync, not `/usr/bin/rsync`.
