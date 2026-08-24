# Design: Direct File Transfer (Rust GUI)

## 1. Overview

A **macOS** desktop application that orchestrates file transfers between computers using **SSH** and **rsync** (Homebrew on macOS). The app installs as a normal macOS application under `/Applications`. Linux machines are **source and/or destination only**—they never run this GUI.

File bytes move **directly from source to destination** when both are remote. They do not stream through the controller (avoid the `rsync user@A:… user@B:…` local-relay pitfall).

Hosts are discoverable via **Avahi / Bonjour** (DNS-SD). Missing SSH keys or peer access **only blocks starting that transfer**; the app always launches and remains usable.

### Roles


| Role            | Name                        | Responsibility                                                                                                 |
| --------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------- |
| **Controller**  | Computer 1 (macOS only)     | GUI in `/Applications`: discover/save hosts, pick locations/files, run `ssh` / `rsync`, show reliable progress |
| **Source**      | Computer 2 (macOS or Linux) | Holds the files; SSH reachable; Avahi/Bonjour on the LAN                                                       |
| **Destination** | Computer 3 (macOS or Linux) | Receives via **rsync over SSH**; needs `sshd` + `rsync` binary, **not** an rsync daemon                        |


Source and/or destination may be the controller Mac itself.

```
                    ssh (list, orchestrate, progress telemetry)
         ┌──────────────► Controller Mac (GUI .app) ◄──────────────┐
         │                                                          │
         │         ssh: start rsync on source                        │
         ▼                                                          │
    Source host  ══════ rsync over SSH (direct) ══════►  Dest host
                 (no rsyncd; dest runs rsync --server via sshd)
```

---

## 2. Goals and non-goals

### Goals

1. Run the GUI on one Mac and transfer files from a second computer to a third.
2. Persist known computers for quick selection.
3. Persist known locations (folders) for quick selection.
4. List files in a source location for selection.
5. Select a single destination location.
6. Allow source or destination to be the controller Mac.
7. Direct source→destination data path (not via the controller).
8. No custom agent on remotes; transfers use **rsync over SSH** only—**no rsync daemon (`rsyncd`)** on any host.
9. Reliable GUI progress for the overall job (as far as reasonably possible).
10. Avahi/Bonjour discovery of hosts.
11. Always run as **File Transfer.app** from `/Applications` (personal use; no public distribution).
12. Privacy: never persist or log transferred **filenames**.

### Non-goals (v1)

- Linux or Windows as controller / initiator.
- Configuring or depending on `rsyncd` / port 873 on any host.
- Multi-destination transfers.
- Full remote shell admin UI.
- Cloud sync, versioning, or conflict resolution.
- Blocking app launch on incomplete SSH setup.
- Apple Developer signing, notarization, or third-party distribution (DMG/App Store).

---

## 3. Prerequisites

Personal use only: build locally and install **File Transfer.app** into `/Applications`. No Developer ID, notarization, or shared installer is required. Always launch the app from `/Applications` (not via `cargo run` as the normal run mode).

### Controller Mac (initiator)

| Requirement | Notes |
|-------------|--------|
| macOS | Controller OS; Bonjour built in |
| Xcode Command Line Tools | `xcode-select --install` (linker, SDKs for building) |
| Rust toolchain | [rustup](https://rustup.rs); used to build the `.app` |
| Homebrew + Homebrew `rsync` | `brew install rsync`; prefer `/opt/homebrew/bin/rsync` or `/usr/local/bin/rsync`, not `/usr/bin/rsync` |
| System `ssh` | Built in; non-interactive access to remotes via keys / `ssh-agent` / `~/.ssh/config` |
| **File Transfer.app** in `/Applications` | Built from this repo (e.g. `cargo bundle` or equivalent) and copied/installed there; this is the supported way to run |

### Source and destination hosts (macOS or Linux)

| Requirement | Notes |
|-------------|--------|
| SSH server (`sshd`) | Reachable from the controller; for A≠B≠C jobs, also reachable peer-to-peer as required by push/pull |
| `rsync` binary | macOS: Homebrew rsync; Linux: distro package. **Not** an rsync daemon |
| Avahi or Bonjour | For `_ssh._tcp` discovery (manual host entry still works if discovery is off) |
| SSH trust | Controller→each host for list/orchestrate; source↔dest keys for direct remote→remote (missing trust only blocks that transfer) |

### Not required

- Installing this GUI on Linux
- `rsyncd` / port 873 anywhere
- Apple Developer account, codesign, or notarization for local `/Applications` use
- Password-interactive SSH during transfers (keys required for Start)

### Quick verification (before first transfer)

1. On the controller: Homebrew `rsync --version` works; `ssh user@host true` succeeds for each remote.
2. For three-machine jobs: from the source host, `ssh` to the destination succeeds (or the reverse for pull).
3. **File Transfer.app** opens from `/Applications` even if some hosts are unreachable.

---

## 4. Platform and tooling

### Controller (initiator)

- macOS only.
- Runs exclusively as **File Transfer.app** from `/Applications` (build locally; no distribution pipeline).
- Locates **Homebrew rsync** for local operations: prefer `/opt/homebrew/bin/rsync`, then `/usr/local/bin/rsync`. Do **not** use `/usr/bin/rsync` when Homebrew is present.
- Uses system `ssh`.

### Source / destination


| Capability              | Required on source | Required on destination             |
| ----------------------- | ------------------ | ----------------------------------- |
| SSH server (`sshd`)     | Yes (if remote)    | Yes (if remote)                     |
| Avahi or Bonjour        | Yes                | Yes                                 |
| `rsync` binary          | Yes                | Yes (for `rsync --server` over SSH) |
| rsync daemon (`rsyncd`) | **No**             | **No**                              |
| This GUI app            | No                 | No                                  |


Transport is always **rsync over SSH** (`rsync -e ssh` / remote shell), never a standalone rsync daemon.

**macOS peers:** use **Homebrew rsync** (`/opt/homebrew/bin/rsync` or `/usr/local/bin/rsync`) as client and, when the Mac is the SSH remote side, pass `--rsync-path=` to that same Homebrew binary. Do not rely on `/usr/bin/rsync`.

**Linux peers:** distro `rsync` package is enough on both source and destination.

### Trust and startup behavior

- Preferred auth: SSH keys / `ssh-agent` / `~/.ssh/config` (non-interactive).
- **App start is unconditional.** Discovery, saved hosts/folders, and UI work without working keys.
- **Transfer start** is gated: if controller↛source, controller↛dest (when needed), or source↛dest (for direct push) fails preflight, disable/block **Start** and show a clear fix hint—do not quit or crash the app.

---

## 5. Discovery (Avahi / Bonjour)

All participating hosts advertise and/or are browsable via DNS-SD.

### Browse

- On the controller, browse **`_ssh._tcp`** only (v1) via macOS Bonjour APIs (`dns_sd` / suitable Rust crate). No app-specific DNS-SD type in v1.
- Present discovered names, hostnames, and addresses in the Computers UI (merge with manually saved entries by stable id / hostname).

### Advertise

- Remotes already expose SSH via Avahi/Bonjour on typical Linux/macOS LAN setups.
- Document that SSH (`_ssh._tcp`) advertisement should be enabled; that is sufficient for discovery.

### Persistence vs discovery

- User can **save** a discovered host (and folders on it) for quick selection offline.
- Discovery refreshes availability badges (online / unreachable) without removing saved records.

---

## 6. User experience

### Primary flow

1. Select **source computer** (saved, discovered, or This Mac).
2. Select **source location** (saved folder or add path).
3. **List files**; multi-select (dirs allowed as units).
4. Select **destination computer**.
5. Select **one destination location**.
6. Preflight access; if OK, **Start**.
7. Progress UI updates from measured bytes / known total; Cancel supported.
8. Completion summary: status, byte counts, duration—**not** a list of filenames on disk.

### Soft-fail access

- Missing keys, host key prompts in batch mode, or peer SSH failure → transfer unavailable with explanation (“Set up key access from Studio → NAS”), rest of app unchanged.

---

## 7. Transfer model (direct path, rsync over SSH)

### No rsync daemon

- Never use `host::module` / port 873 / `rsyncd`.
- Every remote hop is rsync with SSH as the remote shell; the far side runs `rsync --server` under `sshd` (binary required, daemon not).

### Do not relay through the controller

**Wrong** (data flows source → controller → dest):

```bash
rsync -a user@source:/path/ user@dest:/dest/
```

**Right** for remote→remote—prefer **push**: run rsync **on the source**:

```bash
ssh user@source 'rsync -a --files-from=… -e ssh /src/base/ user@dest:/dest/'
```

**Pull fallback** if push preflight fails (source↛dest) but dest→source works:

```bash
ssh user@dest 'rsync -a --files-from=… -e ssh user@source:/src/base/ /dest/'
```

Preflight probes both directions; **prefer push** when both work.

On macOS ends, pass `--rsync-path=/opt/homebrew/bin/rsync` (or `/usr/local/bin/rsync`) for the remote Mac.

### Scenario matrix


| Source   | Dest     | Where rsync client runs                        | Data path                                    |
| -------- | -------- | ---------------------------------------------- | -------------------------------------------- |
| This Mac | This Mac | Controller (Homebrew rsync)                    | Local `rsync -a`                             |
| This Mac | Remote   | Controller                                     | `rsync -e ssh … user@dest:…`                 |
| Remote   | This Mac | Controller                                     | `rsync -e ssh user@source:… …`               |
| Remote A | Remote B | **A** via `ssh A 'rsync … B:…'` (or B on pull) | A ↔ B over SSH; controller only orchestrates |


For **A → B**, the controller only SSHes to the side that runs the client; payload never traverses the controller.

### Selection payload

- Pass selected relative paths via a temporary `--files-from`-style list on the machine running the client (created over SSH to the source when needed).
- That temp list lives only for the job on the remote/controller temp dir and is deleted afterward. **Do not** copy it into app logs or the SQLite store.

### Path layout

- Preserve relative paths under the destination location by default.

---

## 8. Reliable progress (GUI)

`--info=progress2` alone is **not** sufficient: SSH line buffering, sparse updates, skipped files, and multi-file jobs make a single rsync progress line a weak sole signal.

### Approach

1. **Preflight size** — Before transfer, compute `total_bytes` (and optional `total_files`) for the selection:
  - Local: walk metadata.
  - Remote: SSH script summing sizes for selected paths (portable `stat`/`find`; no GNU-only assumptions without detection).
2. **Drive the bar from** `bytes_completed / total_bytes` (clamp 0–1). Show throughput from a short moving window.
3. **Emit measurable progress** — primary: parse rsync progress/itemized output and map onto preflight `total_bytes`. Secondary: an external byte counter only if parsing is insufficient for reliable UI updates.
   - Prefer running the client where stdout can be captured reliably (local controller, or `ssh` with explicit buffering control on the source).
4. **Unbuffered telemetry** — Force line-buffered progress on SSH where possible (`stdbuf` on Linux; avoid TTY-only tricks). Poll the UI on a steady interval so the bar moves across bursty output.
5. **Indeterminate fallback** — If size preflight fails, show indeterminate progress + bytes seen so far, never a stuck 0% bar pretending certainty.
6. **Live filename in UI only** — Optional “current file” in memory for reassurance; **must not** be written to store, job history, or log files (see §10).

### Cancel

- Kill the controller-side process tree (local rsync/ssh). For remote jobs, tearing down SSH should stop the remote rsync; verify on macOS/Linux sources.

---

## 9. Listing files on the source

- Remote: SSH to a portable listing command (NUL-safe or TSV: name, type, size, mtime).
- Local: Rust directory read.
- Do not persist listing results beyond UI session cache as needed for the current screen.

Escape paths safely; prefer argv-style remote commands over `sh -c` string building.

---

## 10. Persistence and privacy (`ft-store`)

### Allowed to persist / log

- Computers (name, SSH destination, discovery ids, ports, identity file refs).
- Locations / folders (name, absolute path, computer id).
- Job **metadata**: timestamps, source/dest **computer ids**, source/dest **location ids or folder paths**, byte counts, duration, success/failure, exit codes.
- App diagnostics that do not include file names.

### Forbidden to persist / log

- Names of transferred files or relative paths of selected entries.
- Contents of `--files-from` lists.
- Rsync itemized output or per-file log lines on disk.

History UI may show “42 files, 1.2 GB, Studio Photos → NAS Inbox, OK”—not individual file names.

### Schema (illustrative)

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

SQLite on the controller Mac only.

---

## 11. Architecture

```
┌─────────────────────────────────────────────────────────┐
│  macOS .app (controller only)                           │
│  ┌──────────┐  ┌──────────┐  ┌────────────┐  ┌───────┐ │
│  │ ft-app   │─►│ ft-exec  │─► ssh + rsync │  │ft-mdns│ │
│  │ GUI      │  │ progress │  │ Homebrew    │  │browse │ │
│  └────┬─────┘  └──────────┘  └────────────┘  └───┬───┘ │
│       └──────────► ft-store ◄────────────────────┘     │
└─────────────────────────────────────────────────────────┘
```


| Crate      | Purpose                                                                     |
| ---------- | --------------------------------------------------------------------------- |
| `ft-app`   | egui (eframe) GUI; progress binding; soft-disable Start             |
| `ft-exec`  | Argv builders; Homebrew `--rsync-path`; size preflight; progress accounting |
| `ft-store` | Computers, locations, job metadata (privacy rules enforced here)    |
| `ft-mdns`  | Bonjour browse for `_ssh._tcp`                                      |


---

## 12. GUI

### Toolkit

**egui (eframe)** for the Mac app; package with `cargo-bundle` or equivalent into **File Transfer.app** and install to `/Applications` (supported run mode).

### Panels

1. **Transfer** — source/dest, file list, single dest folder, progress bar, Start/Cancel (Start disabled until preflight OK).
2. **Computers** — saved + discovered; test SSH; never required for launch.
3. **Locations** — folders per computer; validate path when reachable.
4. **History** — job metadata only (no filenames).

---

## 13. Security and safety

- SSH trust model unchanged; app is a structured front-end.
- Strict path escaping; no log of selected names.
- Overwrite policy confirmed before start.
- Optional free-space preflight on destination via SSH.
- Details panel may show a **redacted** command template (host and folder names OK; omit file list).

---

## 14. Error handling


| Failure                                         | App launch | Transfer                                                |
| ----------------------------------------------- | ---------- | ------------------------------------------------------- |
| No SSH keys / permission denied                 | OK         | Block Start; show fix hint                              |
| Source↛dest peer access                         | OK         | Block Start; hint to set up source→dest keys            |
| Discovery empty                                 | OK         | Manual add still works                                  |
| Size preflight fails                            | OK         | Allow start with indeterminate progress                 |
| Mid-transfer disconnect                         | OK         | Fail job; metadata only in history                      |
| Homebrew rsync missing on controller / Mac peer | OK         | Block affected jobs; point to `brew install rsync`      |
| `rsync` missing on Linux peer                   | OK         | Block Start; ask to install distro `rsync` (not rsyncd) |
| User cancel                                     | OK         | Stop rsync/ssh; record cancelled job                    |


---

## 15. Implementation plan

### Phase 0

- macOS app skeleton → **File Transfer.app** installed under `/Applications`
- Local list + local Homebrew `rsync` copy + byte-accurate progress from preflight

### Phase 1

- Store hosts/folders; privacy-safe job records
- SSH list + This Mac ↔ remote via rsync over SSH
- Soft-fail preflight (app always starts)

### Phase 2

- Remote→remote direct push (and pull fallback)
- Avahi/Bonjour browse + save
- Progress hardened (preflight totals + parsed/measured bytes)

### Phase 3

- Overwrite/space checks, history UI; keep local `.app` → `/Applications` install path simple (no notarization)

---

## 16. Testing

- Progress: large multi-file job shows monotonic bar ≈ wall throughput; SSH hop without relying solely on `--info=progress2`.
- Privacy: assert DB/logs contain no selected filenames after a job.
- Packaging: **File Transfer.app** launches from `/Applications` after a local build (Homebrew rsync present).
- Discovery: Linux Avahi + Mac Bonjour hosts appear.
- Access: remove keys → app starts; Start disabled with message.
- Directness: A≠B≠C payload not relayed via controller.
- No `rsyncd`: transfers succeed with only `sshd` + `rsync` binary on dest.

---

## 17. Decisions

| Topic | Decision |
|-------|----------|
| GUI toolkit | **egui (eframe)** |
| Remote→remote | Probe push and pull; **prefer push** from source when both work |
| Progress signals | Preflight `total_bytes` + **parse rsync output**; external counter only if needed |
| DNS-SD | Browse **`_ssh._tcp` only** in v1 (no custom service type) |
| App display name | **File Transfer.app** (always run from `/Applications`) |
| Distribution | Personal / local build only — no signing or notarization |

---

## 18. Success criteria

- **File Transfer.app** in `/Applications` initiates transfers; Linux never needs the app.
- A→B→C style jobs move data B→C directly via **rsync over SSH** (no `rsyncd`), preferring push from B when possible.
- Progress bar tracks job completion using preflight totals and parsed (or, if needed, externally counted) bytes.
- Hosts discoverable via `_ssh._tcp` (Avahi/Bonjour); hosts/folders persist; **filenames never persisted or logged**.
- Broken SSH setup does not prevent startup—only blocks the affected transfer.
- macOS rsync usage is Homebrew’s binary, not `/usr/bin/rsync`.

