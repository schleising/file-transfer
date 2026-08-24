# Design: Direct File Transfer (Rust GUI)

**Status:** Implemented (personal-use v1). This document describes the **as-built** system in this repo.

## 1. Overview

A **macOS** desktop application that orchestrates file transfers between computers using **SSH** and **rsync** (Homebrew on macOS). The app installs as **File Transfer.app** under `/Applications`. Linux machines are **source and/or destination only**—they never run this GUI.

File bytes move **directly from source to destination** when both are remote. They do not stream through the controller (avoid the `rsync user@A:… user@B:…` local-relay pitfall).

Hosts are discoverable via **Avahi / Bonjour** (`_ssh._tcp`). Missing SSH keys or peer access **only blocks that transfer**; the app always launches and remains usable.

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
3. Persist known **locations** (host + folder path) for quick selection on the Transfer tab.
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

- macOS only; **egui (eframe)** GUI.
- Install: `./scripts/install-app.sh` → `cargo build --release -p ft-app`, assemble minimal `.app`, copy to `/Applications`.
- Local rsync: `/opt/homebrew/bin/rsync` then `/usr/local/bin/rsync` (never prefer system `/usr/bin/rsync` when Homebrew exists).
- Data dir: `~/Library/Application Support/File Transfer/` (SQLite).

### Remotes

- Transport: **rsync over SSH** only.
- Peer and controller SSH options include `BatchMode=yes`, `ConnectTimeout=8`, `StrictHostKeyChecking=accept-new`, `UpdateHostKeys=no`, `ControlMaster=no`, `ControlPath=none`. One-shot controller SSH uses `-n` (stdin closed); rsync `-e ssh` does **not** use `-n` (protocol needs stdin).
- Remote orchestration SSH adds `-tt` so rsync progress is line-flushed through the session; remote rsync uses `stdbuf -oL` when available.
- Remote Mac rsync: prefer Homebrew paths when resolving the client on that host; `--rsync-path=rsync` for the server side of a hop (PATH / package on Linux).

### Trust and startup

- App always starts.
- **Check access** / Start gated on SSH and peer reachability; failures show hints, do not crash the app.

---

## 5. Discovery (Avahi / Bonjour)

- Crate `ft-mdns` browses **`_ssh._tcp.local.`** via the `mdns-sd` crate.
- Computers UI: list discoveries; **Save** into the store.
- Manual add of SSH destinations remains supported.
- No custom DNS-SD service type in v1.

---

## 6. User experience (as built)

### Tabs

1. **Transfer** — source/dest location pickers, file multi-select, progress (rate/ETA), Start/Cancel, Check access.
2. **Computers** — CRUD, Test SSH, Bonjour discoveries.
3. **Locations** — saved folders per computer.
4. **History** — job metadata only (no filenames).

### Folder selection

| Computer | Browse behavior |
|----------|-----------------|
| This Mac | Native folder dialog (`rfd`) |
| Remote | In-app SSH folder browser (list dirs, Up / Home / Go, Select this folder) |

**Locations** are stored as a **computer + folder path** pair. On the Transfer tab, source and destination each have a single quick-select dropdown listing all saved locations as **`Host — Folder (path)`** (not a two-step “pick computer, then pick saved folder”). Selecting an entry sets both the host and folder.

To add or change a path: choose **Browse on** (host picker) → **Browse…** or type a path → **Use path**. New paths are saved as locations and appear in the dropdown. The **Locations** tab still supports explicit CRUD.

### Transfer flow

1. **Source** — pick a saved location (or browse / type path) → **List**.
2. Multi-select files and/or folders (dotfiles hidden in listings).
3. **Destination** — pick a saved location (or browse / type path).
4. **Check access** → **Start transfer** → progress bar with rate and ETA; **Cancel** kills the local ssh/rsync child.
5. UI shows **Transfer complete** as soon as rsync reports payload done (see §8); SSH teardown continues in the background.
6. History records counts/bytes/status only.

### Soft-fail access

Peer host-key or permission failures block the transfer with an explanation (including that the app SSH name must match interactive use). UI otherwise remains usable.

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

1. **Preflight** — size/count of the (expanded) selection when possible (`bytes_total`, `file_count` on the job).
2. **Bar** — prefers rsync’s own **percent** from progress2 when available; otherwise `bytes_done / bytes_total`. Shows **transfer rate** and **ETA** when parsed or derivable. Indeterminate animation if total unknown.
3. **Parsing** — progress2 lines like `bytes  pct  rate  time (xfr#N, to-chk=L/T)`; extract bytes, percent, rate (`KB/s`/`MB/s`/…), and `to-chk` / `ir-chk` for completion detection.
4. **Stream handling** — drain **both stdout and stderr**; split on `\r` and `\n`. On rsync 3.x with piped I/O, progress2 goes to **stdout** (not stderr). Failing to drain either pipe can **block rsync** (historically stderr; stdout null also caused stalls). Use `--outbuf=N` on the client.
5. **Completion semantics** — payload is **done** when rsync reports any of: `100%`, `to-chk=0`, or progress stalls at ≥99% / ≥95% of preflight bytes for ~1.2s. The UI marks the job complete immediately (`data_complete`); the executor returns without waiting for SSH session teardown. A background thread keeps draining pipes and reaps the child (SIGKILL after a few seconds if stuck).
6. **Remote→remote** — orchestration SSH uses `-tt`; remote script prefers `stdbuf -oL` around rsync so progress streams instead of buffering until session exit.
7. UI repaints on background progress messages while a transfer runs.
8. Filenames are not persisted; optional in-memory “current file” is not used for progress2.

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

- Computers, locations, job metadata (ids, folder location ids, byte counts, file counts, status, short error summary).

### Forbidden

- Transferred filenames / selection paths in DB or logs.
- Persisting `--files-from` contents.

### Schema (as built)

```text
Computer {
  id, name, ssh_destination, ssh_port?, identity_file?,
  bonjour_name?, last_seen_at?, is_local, created_at, updated_at
}

Location {
  id, computer_id, name, path, kind: Source | Dest | Either,
  created_at, updated_at
}

JobRecord {
  id, started_at, finished_at?,
  source_computer_id, source_location_id,
  dest_computer_id, dest_location_id,
  bytes_total?, bytes_transferred?, file_count?,
  status, error_summary?   // no filenames
}
```

On first open, a **This Mac** computer and a **Home** location are created automatically.

---

## 11. Architecture (as built)

```
crates/
  ft-app/     egui UI, background jobs, folder pickers, install target binary
  ft-exec/    ssh/rsync, listing, expand folders, progress parse, peer probe
  ft-store/   SQLite computers / locations / jobs
  ft-mdns/    _ssh._tcp browse (mdns-sd)
scripts/install-app.sh   release build → File Transfer.app → /Applications
```

| Crate | Role |
|-------|------|
| `ft-app` | eframe UI; `rfd` native picker; SSH browse window; wires store + exec + mdns |
| `ft-exec` | All process orchestration and progress |
| `ft-store` | Persistence + privacy boundary |
| `ft-mdns` | Discovery snapshot for the Computers tab |

---

## 12. GUI details

- Toolkit: **egui / eframe**.
- Packaging: minimal `Info.plist` + binary `Contents/MacOS/file-transfer` (not cargo-bundle).
- Transfer: combined **host / folder** location dropdowns; separate **Browse on** host picker for native or SSH folder browse; path entry + **Use path** for ad-hoc locations.
- Progress bar: percent (from rsync when available), bytes, rate, ETA; status line **Transfer complete** on payload done.
- Start enabled when source/dest locations are set and at least one entry is selected (preflight still required for a clean run).

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
| Long pause at 100% waiting for SSH teardown | — | Fixed: complete on progress2 done; reap in background |
| Missing Homebrew rsync on controller | OK | Local rsync ops fail with install hint |
| Cancel | OK | Kill child; status cancelled |

---

## 15. Implementation status

| Area | Status |
|------|--------|
| Workspace crates + egui app | Done |
| Store + This Mac / Home defaults | Done |
| Local ↔ remote ↔ remote→remote (push/pull) | Done |
| Bonjour `_ssh._tcp` + save | Done |
| Native + SSH folder browse | Done |
| Progress2 parse + dual-stream drain + `--inplace` + `--outbuf=N` | Done |
| Progress rate/ETA + early UI completion on payload done | Done |
| Transfer tab: combined host/folder location quick-select | Done |
| Folder expand for `--files-from` | Done |
| Install script → `/Applications` | Done |
| Overwrite policy UI / df preflight / redacted command panel | Not in v1 |
| Signing / notarization | Out of scope |

---

## 16. Testing (manual / as used)

- Multi-file and folder transfers with visible progress, rate, and ETA.
- “Transfer complete” appears when payload finishes, not after SSH teardown.
- Remote Mac and Linux listing (bash + Python).
- Peer SSH with matching hostnames / `accept-new`.
- Privacy: DB has no per-file names after jobs.
- Install from `scripts/install-app.sh` and launch from `/Applications`.

---

## 17. Decisions (locked + as-built notes)

| Topic | Decision / as-built |
|-------|---------------------|
| GUI | **egui (eframe)** |
| Remote→remote | Probe both; **prefer push** |
| Progress | Preflight total + parse **progress2 from stdout** (`\r`); `--outbuf=N`; drain both pipes; rate/ETA; **complete UI on payload done** |
| Locations (Transfer tab) | **Single dropdown per side**: `Host — Folder (path)`; browse/path adds saved locations |
| DNS-SD | **`_ssh._tcp` only** |
| App name / run mode | **File Transfer.app** in `/Applications` |
| Distribution | Personal build; `scripts/install-app.sh` (no cargo-bundle, no notarization) |
| Folder pick | **rfd** local; **SSH browser** remote |
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
- Host/folder locations persist as combined entries; filenames never stored.
- Broken SSH setup does not prevent startup.
- Controller uses Homebrew rsync, not `/usr/bin/rsync`.
