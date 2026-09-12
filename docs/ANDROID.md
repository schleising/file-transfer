# Design: Android controller on the LAN

**Status:** Assessment only (not implemented). The as-built Mac app remains [DESIGN.md](DESIGN.md). This document asks whether a **personal, sideloaded Android APK** can run the same transfer flow while the phone is on your Wi-Fi, including when the app is backgrounded or the screen is locked.

**Verdict:** Yes, as a **second controller** on the same LAN — not as a Play Store product, and not by cloning the macOS process model. The wizard, store, discovery, and `ft-exec` transfer model can be reused. Installation, process lifetime, SSH/rsync, and layout all need Android-specific work. The hard parts are **bundled binaries + a transfer-scoped foreground service**, not the Dioxus UI itself.

---

## 1. Overview

The Mac app is a **controller**: it lists folders, plans a job, and runs `ssh` / `rsync`. Payload still goes **source → destination directly** when both are remote. Android can play that same role from the couch or while the phone is in a pocket, as long as it is on the **same local network** as the SSH hosts.

Android is **not** a drop-in for `/Applications/File Transfer.app`:

| Mac today | Android equivalent |
|-----------|-------------------|
| Always-on process + menu-bar extra | Process dies unless a **foreground service** is running |
| System OpenSSH + Homebrew rsync | **Bundle** `ssh` and `rsync` (or give up local↔remote) |
| 1280×840 sidebar + main | **Stacked phone layout**; same Source → Files → Destination → Transfer |
| Close hides; Open at Login | No always-running extra; start the app when you need it |
| `~/Library/Application Support/…` | App-private files dir + optional shared storage |

Linux and macOS hosts stay **source and/or destination only**. They do not run an Android-specific agent. The phone is **not** advertised as `_ssh._tcp`; the Mac cannot pick “the phone” as a remote unless we later add an SSH server on the device (out of scope).

```
                    ssh (list, orchestrate, progress telemetry)
         ┌──────────────► Android phone (GUI APK) ◄──────────────┐
         │                      or the Mac GUI                    │
         │         ssh: start rsync on source (push)               │
         ▼                                                        │
    Source host  ══════ rsync over SSH (direct) ══════►  Dest host
                 (no rsyncd; dest runs rsync --server via sshd)
```

“This Phone” as a **local** source or destination is in scope (photos off the device, files onto it). That is the case that needs a local `rsync` binary. Remote→remote jobs need only an SSH client on the phone.

---

## 2. Goals and non-goals

### Goals

1. Sideload a personal APK (USB/`adb` or an APK file). No Play Store.
2. Same user flow as the Mac: **Source → Files → Destination**, automatic preflight, **Transfer** / **Cancel**, progress with rate and ETA, 5s auto-reset, Last transfer in the chrome.
3. Orchestrate LAN jobs while the phone is on Wi-Fi, including **app not in the foreground** and **screen locked**.
4. Status-bar **ongoing notification** while a transfer runs; a **completion** (and failure/cancel) notification when it ends.
5. Phone-sized UI that keeps the current steps, tiles, file list, sheets, and footer actions — restacked, not redesigned into a different product.
6. Optional **This Phone** locations (device storage as source and/or dest), using the same in-app folder browser.
7. Stay on the LAN only. No WAN, relay, or cloud.

### Non-goals

- Play Store, Play App Signing, F-Droid, or public distribution.
- Google Play policy compliance (`MANAGE_EXTERNAL_STORAGE`, background restrictions as a store app, etc.).
- Cellular / off-network transfers; Tailscale/VPN as a requirement (if a VPN happens to expose LAN SSH, that is incidental).
- Android as an SSH **server** that the Mac discovers.
- Always-on daemon, boot receiver, or “Open at Login” equivalent.
- iOS (background + binary-exec story is worse; not assessed here).
- Rewriting the rsync protocol in Rust, or replacing rsync with SFTP as the primary data plane.
- Sharing the Mac SQLite file with the phone (separate store; re-add hosts).
- Material You redesign, Android share-sheet as the main entry, or a Kotlin UI rewrite.

---

## 3. What has to change (gap analysis)

`ft-store`, `ft-mdns`, and most of `ft-exec` are host-agnostic **if** `ssh` and `rsync` exist as child processes. `ft-app` is not: it launches Dioxus **desktop**, restores a window frame, and attaches a macOS extra (`crates/ft-app/src/macos.rs`).

### Keep

- Wizard, location tiles, in-app browser, preflight gate, progress2 parsing, `--inplace` / `--files-from` / folder expand, privacy rules, Bonjour `_ssh._tcp` plus manual hosts.
- Transfer modes: local↔local, local↔remote, remote→remote push (preferred) / pull.

### Replace or add

| Area | Mac | Android |
|------|-----|---------|
| Package | `File Transfer.app` via `install-app.sh` | Signed **APK** via `install-android.sh` + `adb` |
| UI shell | `dioxus` desktop + WKWebView | Dioxus **mobile** + Android WebView (same RSX) |
| Layout | 248px sidebar + main, min 900×560 | Single column; steps in the top bar; summary collapsed |
| Background | Hide window; process stays | **Foreground service** for the duration of the job |
| Progress while away | Menu-bar template icon | **Ongoing notification** + progress bar |
| Job done | Sidebar Last transfer (window may be hidden) | **Notification**; Last transfer when the UI is next shown |
| `ssh` / `rsync` | PATH / Homebrew | **Shipped in the APK**, executed from `nativeLibraryDir` |
| Keys | `~/.ssh` + agent | App-private `.ssh`; import a key once |
| Local files | POSIX home | Shared storage with **All files access** (personal) |
| mDNS | `mdns-sd` as-is | Same crate **plus** Wi-Fi multicast lock + Nearby/local-network permission |
| Data dir | `dirs::data_dir()` | Android app files dir (JNI / `ndk-context`) |

Desktop-only APIs that must be `cfg`’d out: tray, native menu, `WindowCloseBehaviour`, `window().drag()`, titlebar drag, `settings.window.frame`.

---

## 4. Recommended architecture

One workspace, two shells, shared crates.

```
crates/
  ft-app/        Shared Dioxus UI + state (desktop + android features)
  ft-exec/       ssh/rsync orchestration (binary paths become platform-specific)
  ft-store/      SQLite (data-dir resolution per OS)
  ft-mdns/       _ssh._tcp browse
android/         Gradle wrapper / manifest overlays (or dioxus.toml generated project)
scripts/
  install-app.sh
  install-android.sh   # dx bundle --platform android && adb install -r
```

**Do not** fork the UI into a Kotlin app. The flow is the product; Dioxus 0.8 already targets Android (`dx bundle --platform android` → APK). Platform glue (service, notifications, multicast lock, permissions) is a small Kotlin/JNI layer next to the Rust runtime, in the **same process**.

Feature split inside `ft-app`:

- `desktop` — current Mac launch path (unchanged for daily use).
- `android` — mobile launch, no tray, notification + service hooks.

`[workspace.package].version` stays the **Mac app** version. An Android APK uses the same `ft-app` version unless we later decide it is a separate product; until then one version string is enough.

### Why not the alternatives

| Approach | Why not (for this repo) |
|----------|-------------------------|
| Controller-only, no This Phone | Simpler binaries (SSH only), but pulling photos off the phone is the obvious phone use. Include local rsync. |
| Pure-Rust SSH (`russh`) + SFTP | Would reimplement listing, progress, and remote→remote orchestration; two data planes. |
| Termux as a dependency | Extra app, extra PATH, worse UX; fine as a **bootstrap** to copy binaries while bringing up the APK, not as the product. |
| Kotlin UI + UniFFI to `ft-exec` | Keeps exec, throws away the wizard. Larger rewrite than restacking CSS + cfg. |
| Mac-style always-running process | Android will kill it. Fight the platform and transfers die with the screen off. |

---

## 5. Installation (not Play Store)

Personal distribution is **sideloading**. That is enough on your own phone and matches how the Mac app is installed today (local build, no Developer ID).

### Build machine (the Mac)

- Rust `aarch64-linux-android` (plus `x86_64-linux-android` only if using the emulator).
- Android SDK + NDK, JDK 17, `dx` CLI matching the Dioxus 0.8 line.
- USB debugging or wireless debugging on the phone.

### Install paths (both OK)

1. **Preferred:** `./scripts/install-android.sh` → release APK → `adb install -r`. Same gesture as `install-app.sh`.
2. **No cable:** copy the APK (Nearby Share, USB storage, or a file already on the LAN) and install with **Install unknown apps** allowed for that source.

Updates use the **same signing key** so `adb install -r` / package installer can replace the app. Use a local keystore in the repo’s gitignore (or a password manager), not the throwaway debug key, so a rebuilt laptop does not brick updates.

Package id: `local.filetransfer` (or `local.file.transfer`) — parallel to Mac `local.file-transfer`. Not a Play `com.google` id.

Min SDK: **26** is enough for a personal phone; target the current SDK so foreground-service types compile. No 32-bit ABI (`armeabi-v7a`) unless a specific old device needs it.

### Permissions to declare

| Permission | Why |
|------------|-----|
| `INTERNET` | SSH |
| `ACCESS_NETWORK_STATE` | Wi-Fi vs cellular check |
| `ACCESS_WIFI_STATE` | Confirm STA Wi-Fi |
| `CHANGE_WIFI_MULTICAST_STATE` | mDNS |
| `NEARBY_WIFI_DEVICES` (13+) / location fallback | Local network / NSD on some OEMs |
| `POST_NOTIFICATIONS` (13+) | Progress + completion |
| `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_DATA_SYNC` | Job while locked |
| `WAKE_LOCK` | Keep CPU/Wi-Fi during the job |
| `MANAGE_EXTERNAL_STORAGE` | POSIX-like browse of `/storage/emulated/0` (personal; not Play-safe) |

First launch: a short setup screen (notification permission, battery optimization exemption, All files access, optional key import). Do not block the rest of the UI the way we do not block Mac launch on SSH — but **Transfer** still waits on preflight, and local browse waits on storage permission.

Play Protect may warn on unknown sources; tap through. No notarization analogue.

---

## 6. Data plane: SSH and rsync on Android

`ft-exec` is built around `Command::new("ssh")` and a Homebrew rsync **3.x** with `--info=progress2` and `--outbuf=N`. Android has neither on PATH for a normal app.

### Bundle statically linked (or NDK-linked) binaries

Ship:

- OpenSSH `ssh` (client only; no `sshd`)
- rsync **3.x** (same flags as the Mac)

Package them as native libraries (`lib/arm64-v8a/libssh.so`, `librsync.so` — the `.so` name is how Android 10+ still allows `execve` from `nativeLibraryDir`). `ft-exec` resolves those paths instead of `/opt/homebrew/bin/rsync`. Set `HOME` (and `UserKnownHostsFile` / `IdentityFile` if needed) to the app files dir so OpenSSH never looks at a missing `/data/home`.

Android 10 blocked executing from the **writable** data dir; executing extracted native libs is the Termux-style workaround and is acceptable for a personal app.

Remote→remote: the phone only SSHs to the source (or dest) and runs rsync **there**, same as the Mac. Local↔remote and This Phone listing/copy need the bundled rsync.

Build of those binaries can live in `scripts/android-bins.sh` (NDK cross-compile, or vendor known-good static builds). Treat them as third-party blobs with versions pinned in the script; do not require Homebrew-on-Android.

### SSH trust

The Mac relies on your existing `~/.ssh`. The phone will not have it.

1. First-run: **import** `id_ed25519` (SAF file picker or paste). Store under the app files dir, mode 0600 as far as the FS allows.
2. Use that identity for every host (`-i` is already in `ft-exec`; the Android shell always sets it).
3. Keep `BatchMode=yes`, `StrictHostKeyChecking=accept-new`, `UpdateHostKeys=no`, `ControlMaster=no`.
4. Put the phone’s **public** key in `authorized_keys` on LAN hosts (once). Using a **phone-specific** key is better than copying the Mac private key, but copying works for a single-user LAN.

No in-app password prompts (BatchMode). No ssh-agent unless a later patch needs it.

Expose a small **Settings** surface that the Mac never needed: which key is loaded, “replace key”, maybe a one-line “not on Wi-Fi” warning. Do not add a Computers admin tab; hosts still come from Add Location.

### This Phone storage

Keep **absolute paths** so the folder browser and `--files-from` stay dumb POSIX.

- Seed computer name **This Phone** (not This Mac); default location **Internal storage** → `/storage/emulated/0` once All files access is granted.
- Optional extra tiles: `DCIM`, `Download`, by browsing as on the Mac.
- App-private files remain available even if All files access is denied; preflight explains the miss.

Scoped-storage-only (SAF trees) would break path-based locations; skip it for v1 Android.

Local↔local on the phone (copy between two folders on device) can use bundled rsync the same way the Mac uses Homebrew rsync for Mac-local copies.

---

## 7. Background running (app not visible, screen locked)

This is the part that does **not** map from macOS.

On the Mac, closing the window **hides** it; the process and the rsync child keep running. Android treats a background Activity as killable. Doze and OEM battery savers will freeze sockets. The Mac extra / LaunchAgent pattern would fail here.

### Transfer-scoped foreground service (required)

When **Transfer** starts:

1. Start an Android **foreground service** (`dataSync`) in the **same process** as the Dioxus runtime.
2. Post the mandatory ongoing notification (see §8).
3. Take a **WifiLock** (and a modest CPU wake lock) for the job.
4. Existing `std::thread` in `AppState` still runs `ft_exec::run_transfer`; the service’s only job is **keep the process alive** and own the notification.

When the job ends (success, fail, cancel):

1. Update / replace with a completion notification.
2. Stop the service.
3. Drop the wifi lock.
4. UI auto-reset timer may still fire if the Activity is alive; if the process was only alive for the service, the next cold start shows Last transfer **None** unless we persist that one field (optional; Mac Last transfer is session-only — keep that unless it feels wrong on a phone).

**Recommendation:** keep Last transfer **session-only** like the Mac. Completion is the notification. Do not invent a job history table.

### What this does not do

- Survive **Force stop**, reboot, or the user dismissing a stoppable service (make the FGS notification not swipe-dismissible while transferring; Cancel stays in-app and as a notification action).
- Run forever in the background waiting for work. No listener, no boot start.
- Beat a 6-hour `dataSync` cap on newer Android versions. LAN copies should be well under that; if not, fail clearly and say to keep the app open.

### User one-time settings (document in the setup screen)

- Disable battery optimization for File Transfer (Settings → Apps → Battery).
- Allow notifications.
- Keep **Wi-Fi** on; do not start a transfer on cellular. If the radio is not STA Wi-Fi, disable **Transfer** with “Connect to your network” (same spirit as Access status).

OEM killers (Xiaomi/Samsung “sleeping apps”) can still murder FGS. For one personal phone this is a checklist item, not an engineering project.

### Dioxus lifecycle

The WebView Activity can be destroyed while the process lives. Rust `AppState` in that process can keep the transfer. When the user returns, the Activity reconnects to the same state **if** we do not tear down the runtime with the Activity.

**Implementation constraint:** the foreground service must be started in a way that Dioxus/JNI does not drop the runtime when the Activity `onDestroy`s. If the 0.8 Android template ties runtime lifetime to the Activity, fix that in the Android shell before relying on lock-screen transfers. This is the highest technical risk in the port; prove it with a dummy long `sleep` job before polishing UI.

---

## 8. Notifications and status-bar icons

Android has **no menu-bar extra**. The replacement is the notification shade + a small **status-bar icon** from the foreground-service notification.

### Channels

| Channel | Importance | Use |
|---------|------------|-----|
| `transfers-ongoing` | Low (no sound, no heads-up) | In-progress FGS notification |
| `transfers-done` | Default (sound/vibrate per user) | Complete / failed / cancelled |

### In progress (ongoing)

- Status-bar **small icon**: white silhouette derived from the existing app mark (arrows). This is the analogue of the Mac extra’s busy ring.
- Title: `File Transfer`
- Text: percent, rate, ETA (same numbers as the footer). No filenames (privacy).
- `setProgress(100, pct, indeterminate)` so the shade shows a bar.
- Actions: **Cancel** (sets the existing `cancel` flag), **Open**.
- `ongoing` / `setForeground` so it cannot be swiped away mid-job.
- Update on the same cadence as the UI poll (~100ms is too chatty for `NotificationManager`; throttle to **~1s** or on percent change).

The in-app footer still updates when the Activity is visible. The notification is not a second progress implementation — it reads the same `Progress` struct.

### Complete / failed / cancelled

- Auto-cancel, tap opens the app.
- Title/text: `Transfer complete` / `Transfer failed` / `Transfer cancelled` plus byte count when known. No paths or names.
- Does **not** use the FGS; posted after the service stops.

Idle app: **no** status-bar icon. That is an intentional difference from the Mac extra, which exists because the desktop process is always running.

---

## 9. UI for a phone screen (keep the flow)

Do not invent new steps. Restack the existing chrome so it fits a ~390×844 logical viewport and fat-finger targets.

### Layout

Mac: sidebar (brand, Source/Files/Destination, Summary, Access, Last transfer, Reset) + main + footer.

Phone:

```
┌─────────────────────────────────┐
│  [icon] File Transfer      ⋮    │  top app bar (safe area)
│  Source · Files · Destination   │  same three steps, as pills
├─────────────────────────────────┤
│                                 │
│   host groups + location tiles  │  existing tile visual, wrap
│   or file list                  │
│                                 │
├─────────────────────────────────┤
│  Summary          Accessible ▾  │  collapsed; tap expands
│  Last transfer    Complete      │  same fields as the sidebar
├─────────────────────────────────┤
│  62% · 45 MB/s · 1m left        │  existing footer content
│  [Cancel]            [Transfer] │  Continue/Back in wizard bar
└─────────────────────────────────┘
```

- **Steps** stay Source / Files / Destination. The selected pill is the current `NavTab`. Locked while preflight/transfer, as today.
- **Continue / Back** stay on the wizard bar (above the footer or in the page chrome).
- **Reset** moves into the ⋮ menu with About / key setup — it is not worth a persistent sidebar button on a short screen.
- **Summary / Access / Last transfer** become a **collapsible strip** above the footer (default collapsed to one line each). Expanding shows the same copy, including the file-count label. Mac **hover** lists (selected names, crate versions) become **tap**.
- **Tiles:** keep 118×108 tiles; they already wrap. On a 390px screen that is three per row — acceptable. Increase tap slop; keep drag-reorder (touch).
- **File list:** full width rows; larger hit area (~44px). Select All / Clear / Refresh stay.
- **Sheets** (Add Location, folder browser): **full-screen** on a phone (not a 560px centered card). Same fields and Browse / New Folder behavior.
- **Footer:** full width (today it is `grid-column: 2`). Stack meta above the bar if the rate/ETA string wraps.
- Drop titlebar drag, traffic-light spacer, and `-webkit-app-region`.
- Type: keep the current palette and 13px density as far as possible; bump nav/pills to 15px if tap tests fail. Use `system-ui` (Android will not have SF Pro).
- Safe areas: `env(safe-area-inset-*)` for notch and gesture bar.
- Light/dark: existing `prefers-color-scheme` already works in WebView if the system theme is followed.

Implementation: `android.css` (or a `@media (max-width: 700px)` block shared with a future small window) plus a few `cfg` / `is_android` branches in `ui.rs` for structure (sidebar vs top pills). Do not duplicate the wizard state machine.

### What we accept as different

- No menu-bar busy glyph when idle.
- No window geometry restore.
- Hover-only chrome is tap-only.
- About crate versions behind a tap on `vX.Y.Z`, not hover.

---

## 10. Discovery and “local network only”

`ft-mdns` (`mdns-sd`, browse `_ssh._tcp.local.`) can stay. On Android it needs:

1. `CHANGE_WIFI_MULTICAST_STATE` and a **`MulticastLock`** held while the Add Location sheet is open (same lifetime as today’s discovery daemon).
2. Runtime **Nearby devices** (or location on older OEMs). If denied, the sheet still supports **Add host manually** — already a first-class path.

Some phones drop mDNS anyway. That is acceptable; you already live with manual hosts.

**Wi-Fi only:** if `ConnectivityManager` says the active network is not Wi-Fi (or is VPN-only with no LAN), show Access-style detail and do not start transfers. No attempt to use mobile data as a WAN path.

The phone does **not** publish `_ssh._tcp`. Discovery remains “find my NAS and Macs,” not “find this phone from the Mac.”

---

## 11. Persistence, privacy, identity

SQLite schema stays. `app_data_dir()` on Android must be the app files directory (not a synthetic `dirs::data_dir()` that may be wrong under JNI).

Still forbidden: transferred filenames in the DB or logs; job history; persisting `--files-from`.

New allowed settings (small KV, like `window.frame` on Mac):

- Notification permission already-asked flag (optional).
- Path to imported identity file (or just a fixed `files/.ssh/id_ed25519`).

Do not sync the Mac DB to the phone.

---

## 12. Security and safety

Unchanged trust model: **your SSH keys, your LAN, rsync overwrite defaults**.

Extra Android notes:

- Sideloaded APK: you are the only installer; keep the keystore private.
- Bundled `ssh`/`rsync` are attack surface; pin versions, do not download them at runtime.
- `MANAGE_EXTERNAL_STORAGE` is powerful; personal-only is why it is acceptable.
- WebView UI is local (no remote HTML). Continue to treat paths as data, not shell, via existing `shell_quote`.
- `accept-new` host keys: first connection to a LAN host still TOFU. Fine on a home network.

---

## 13. Risks

| Risk | Mitigation |
|------|------------|
| Dioxus runtime dies with the Activity → transfers die when you leave the app | Prove FGS + same-process runtime **first** (spike) |
| Cannot `exec` bundled binaries on a given API level | `.so` in `nativeLibraryDir` + `extractNativeLibs=true`; spike on the actual phone |
| OEM battery saver kills FGS | One-time exemption; document; fail visibly if the child is SIGKILL’d |
| mDNS empty on the phone | Manual hosts; multicast lock |
| rsync 3.x Android build missing progress2 / `--outbuf=N` | Same parse path as Mac; reject ancient rsync at startup |
| `-tt` PTY allocation fails in the app sandbox | Test remote→remote early; fall back to `stdbuf` only if needed |
| All files access UX | Setup screen; app-private fallback |
| Dioxus 0.8 alpha Android template churn | Pin `dx` CLI; isolate Gradle overlays so Mac desktop keeps working |
| WebView + `100vh` / keyboard covering sheets | Full-screen sheets; `visualViewport` if the browser sheet is covered |

The Mac app must remain the daily driver until the spike in §15 passes. Do not couple Mac packaging to Android Gradle.

---

## 14. Suggested implementation phases

Do these in order. Later phases are wasted if the process model or binaries fail.

0. **Spike (go/no-go)** — empty Dioxus Android APK on the real phone; FGS that keeps a Rust thread running for 10+ minutes with the screen off; `exec` of a bundled `rsync --version` and `ssh -V`. If either fails hard, stop and reconsider SFTP/`russh` rather than inventing a desktop-style background hack.

1. **Shell** — `install-android.sh`, signing, permissions, setup screen, Wi-Fi gate.

2. **Exec port** — `ft-exec` binary discovery + `HOME`/identity; SSH to an existing LAN host; listing + preflight.

3. **Notifications** — FGS ongoing progress + completion; Cancel action.

4. **UI restack** — pills, collapsible summary, full-screen sheets, footer; keep state machine.

5. **This Phone** — All files access, default location, local↔remote copies.

6. **Polish** — mDNS multicast lock, battery-optimization deep link, About, icon in the status bar matching the Mac mark.

No crate version bump until code ships (this document is assessment only).

---

## 15. Success criteria

- APK installs via `adb` (and optionally the system installer) and launches without Play Store.
- Source → Files → Destination → Transfer matches the Mac semantically on a phone screen.
- A remote→remote job started on the phone **finishes after Home / lock**; status bar shows in-progress; a completion notification appears; payload did not relay through the phone.
- Cancel from the notification stops the job.
- This Phone → NAS (or the reverse) copies real files with progress2 rate/ETA.
- Leaving Wi-Fi refuses Transfer rather than hanging on cellular.
- Mac app behavior unchanged.

---

## 16. Proposed decisions (not locked until implementation)

| Topic | Proposal |
|-------|----------|
| Role | Second **controller** on the LAN; optional **This Phone** source/dest |
| Distribution | Sideload APK; local keystore; no Play Store |
| UI toolkit | Same Dioxus 0.8 app, `android` feature + restacked CSS |
| Data plane | Bundled OpenSSH client + rsync 3.x; keep `ft-exec` |
| Background | Foreground service for the **job only**; no always-on extra |
| Status UI | Ongoing FGS notification (bar + status-bar icon); done channel for complete/fail/cancel |
| Layout | Top step pills + collapsible summary + full-width footer; same wizard |
| Network | Wi-Fi LAN only; mDNS best-effort |
| Keys | Import once into app-private `.ssh` |
| Storage | All files access + POSIX paths |
| History | Still none; Last transfer session-only |
| Mac SSH server on the phone | Out of scope |

---

## 17. Open questions (only if the spike succeeds)

1. Phone-specific SSH key vs reuse the Mac key (operational preference).
2. Whether completion should persist Last transfer across process death (Mac does not).
3. Whether a future Mac build should ever **push to the phone** (requires `sshd` on Android — a different design).
4. Exact Dioxus 0.8 Android template vs a hand-maintained Gradle project, once `dx bundle` is tried on this workspace.
