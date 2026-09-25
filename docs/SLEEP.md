# Design: Transfers when the controller disconnects

**Status:** Not implemented. The as-built Mac app remains [DESIGN.md](DESIGN.md). This document specifies what happens when the **controller Mac** loses its network or sleeps — the usual case is closing the laptop lid — and how the same job picks up when the Mac is back.

Closing the window does not do this. The process already hides to the menu-bar extra and keeps the SSH session. Sleep is different: the process is frozen, TCP dies, and today’s SSH session is what keeps rsync alive.

---

## 1. Overview

Two different machines are involved, and they do not fail the same way.

| Who holds the bytes | While the controller is asleep or offline | When it wakes or reconnects |
|---------------------|--------------------------------------------|-----------------------------|
| **Remote → remote** (push or pull) | rsync keeps running on the runner host. The controller is not on the data path. | Reattach for progress and the exit code. If that rsync died, start it again. |
| **Controller is source or destination** | Nothing moves. The Mac is frozen, and the SSH transport under local rsync is dead by the time it wakes. | Start the same rsync again. Finished files are skipped; the interrupted file is sent again. |
| **Local → local** | The copy is frozen with the Mac. | The same rsync process continues. Do not start a second one. |

The app does **not** prevent sleep. No `caffeinate`, no `IOPMAssertion`. The lid is allowed to close.

The signal is **system sleep and wake**, not the lid angle. Lid close sleeps a laptop unless clamshell mode keeps it awake (power, external display, external input). In clamshell mode the existing session keeps running and nothing has to recommence. The same reconnect path covers Apple-menu sleep, idle sleep, and the controller losing Wi-Fi while the lid stays open. “Lid closed” is the case that must work; the mechanism is a dead controller session.

```
Remote → remote, lid closed:

  Controller (asleep)          telemetry SSH dies
  Source host  ════ rsync over SSH (still running) ════►  Dest host

Lid open:

  Controller ── new telemetry SSH ──► runner
       still running → show progress
       exit 0        → Transfer complete
       gone          → rsync again (finished files skipped)
```

---

## 2. Goals and non-goals

### Goals

1. A transfer in progress survives the controller sleeping or losing the network. It is not marked **Failed** or **Cancelled** for that alone.
2. Remote→remote payload keeps moving while the controller is gone.
3. When the Mac wakes, or the link returns, the app recommences on its own: reattach, or run rsync again from the in-memory plan.
4. **Cancel** still stops the job, including an rsync that was left running on a remote runner.
5. Filenames stay out of SQLite and out of logs. The plan lives in the transfer thread’s memory, which sleep does not wipe.

### Non-goals

- Keeping the Mac awake while the lid is closed.
- Resuming after **Quit** or a relaunch. The selection is not a job record. Quit best-effort kills a detached remote rsync; if the controller is already offline, that rsync may finish unattended and the next launch will not pick it up.
- Block-level append of a half-written file (`--append-verify`). See §6.
- A second machine taking over as controller.
- Changing local→local into a background daemon. The same process simply thaws.

---

## 3. What happens today

`run_transfer` holds one child for the whole job:

- Controller-endpoint modes spawn local Homebrew rsync, which opens SSH itself.
- Remote push/pull runs `ssh -tt` on the runner and that shell runs rsync. Cancel kills the local child; the remote command dies when the session drops ([DESIGN.md](DESIGN.md) §8).

On sleep the app is frozen. The remote sshd times out the dead TCP session and hang-up kills that shell, so remote rsync dies too. On wake the local child errors (`broken pipe`, `connection reset`, ssh exit 255). The transfer thread reports **Failed**, then the five-second plan reset runs.

`--inplace` leaves a short file at the real destination path. `-a` sets mtime when a file finishes, so a later rsync skips completed files and resends the short one. That is already enough to recommence. Nothing in the store has to change.

---

## 4. Session model

One transfer thread owns the plan until success, cancel, or a fatal rsync error. Disconnect is not the end of that thread.

```
start_transfer
  └─ loop
       run one attempt (attached local rsync, or detached remote + telemetry)
       success            → TransferDone Ok, then today’s 5s reset
       cancel             → TransferDone cancelled
       fatal              → TransferDone Err
       transient / sleep  → status “Reconnecting…”, retry the same plan
```

`transferring` stays true across retries, so the wizard stays locked and **Transfer** cannot start a second job. Last transfer stays **In progress**. The five-second reset waits until the job actually ends.

Sleep notifications come from `NSWorkspaceWillSleepNotification` and `NSWorkspaceDidWakeNotification`, registered beside the menu-bar extra in `ft-app`’s `macos.rs` (`objc2`). They set a flag the transfer thread already waits on. `ft-store` is unchanged: no job table, no selection on disk.

### Retry rule

Retry while the controller session died or the network is down. Stop on cancel, success, or a real rsync failure (permission, disk full, missing rsync, a bad host key, partial transfer exit 23).

Treat these as transient:

- ssh exit 255, rsync exit 10 (socket I/O) or 30 (timeout)
- stderr containing connection reset, broken pipe, timed out, no route to host, network unreachable, connection refused, or connection closed

There is no awake-time deadline. A host that is gone stays **Reconnecting…** until the user hits **Cancel**. The footer names the host it is waiting on so this does not look like a hung bar.

After **wake**, wait until a one-shot `ssh … true` succeeds before starting work again. Wi-Fi and DHCP often need a few seconds. Give up that wait only if the user cancels. The first attempts after wake will often fail; that must not become **Failed**.

---

## 5. Remote → remote: detach, then reattach

Detach at **start**, not in the will-sleep handler. Will-sleep is a short window and the radio may already be down. The spawning SSH must exit before the laptop sleeps, so hang-up has no session left to kill.

### Start

1. Upload the `--files-from` list as today (`/tmp/ft-job-<uuid>/list` on the runner).
2. A short SSH starts a wrapper and prints its pid, then exits 0. The wrapper double-forks into a new session (`python3` / `python` `os.setsid`, already the preferred remote tool) so sshd’s SIGHUP does not reach it. If Python is missing, `bash` with `trap '' HUP`, a background process, and `disown` is the fallback.
3. The wrapper runs the same rsync as today (`-a -r --inplace --info=progress2 --outbuf=N --out-format=%i`, Homebrew rsync on the runner when present, `stdbuf -o0` when present so the progress file updates). It writes:
   - `pid`
   - `progress` (stdout)
   - `err` (stderr)
   - `exit` (rsync’s status, written when it finishes)
4. A **second**, disposable SSH tails `progress` and exits when `exit` appears. That session is telemetry only. It does not use `-tt`. When it dies, rsync keeps going.

The exit **file** is the success signal, not the telemetry SSH status. Telemetry dying with 255 while `kill -0` on the pid still succeeds is a reconnect, not a failed transfer.

### On wake or dropped telemetry

1. SSH to the runner. If the job directory is gone, the job is dead (reboot clears `/tmp` on macOS and on many Linux hosts).
2. If `exit` exists: `0` → success; anything else → fatal, surface stderr as today.
3. If the pid is alive and its command name is `rsync` or `stdbuf`: open telemetry again and keep painting progress. The bar may jump forward by the bytes copied during sleep.
4. Otherwise start a new job directory from the same in-memory plan. Remove the dead directory so `/tmp/ft-job-*` does not leak.

Do not launch that new rsync while `kill -0` still sees the old one.

---

## 6. Controller is an endpoint

Local rsync stays a child of the app, as today. On will-sleep, **do not kill it**. Killing a healthy `--inplace` writer truncates the last file; the process is about to freeze anyway.

On wake the TCP session is dead. Wait for that child to exit, then classify the error. If it is still up but has produced no progress for about 15 seconds **and** the mode uses SSH, kill it and treat the attempt as transient. Do not apply that stall-kill to local→local, which is only waiting on disk.

The retry is a new `run_transfer` of the same `TransferPlan` (same relative paths, no second folder expand, no preflight that unlocks the wizard).

**Resume model:** `-a --inplace` only. Completed files match size and mtime and are skipped. The interrupted file is shorter and has a new mtime, so it is sent again from the start. A multi-gigabyte file cut at 99% costs that file again. That is the trade.

`--append-verify` is rejected. It would append onto a shorter destination file of the same name instead of replacing it, and it would do that on every transfer, not only after sleep.

Local→local never enters the retry loop if the original child is still running after wake.

---

## 7. UI

| Moment | Footer | Sidebar Last transfer |
|--------|--------|------------------------|
| Copying, controller awake | Transferring…, rate, ETA | In progress |
| Asleep | (frozen) | In progress |
| Awake, link not ready, or telemetry dropped | Reconnecting to &lt;host&gt;… | In progress |
| Remote job finished during sleep | Transfer complete / Failed | Complete / Failed, then today’s 5s reset |
| User cancels | Cancelled | Cancelled |

Seed the bar from the last `bytes_done` so reconnect does not flash empty. A controller-endpoint retry may then dip: the new rsync recounts and skips finished files before progress2 climbs again. That dip is expected.

The menu-bar ring stays in the transferring state until the job ends. **Cancel** stays enabled during reconnect.

---

## 8. Cancel and quit

**Cancel** sets the existing flag. If a local child is running, kill it as today. If a remote pid is known, SSH and kill that process group, retrying briefly while the network is still coming up. Cancel during **Reconnecting…** means do not start another attempt.

**Quit** performs the same remote kill with a short timeout so quitting does not hang. If SSH is already dead, the detached rsync is left running and will finish on its own. The next launch does not resume it.

---

## 9. Privacy

Same boundary as [DESIGN.md](DESIGN.md) §10.

- The file list stays in the transfer thread and in `/tmp` on the runner (and the controller’s existing temp list). Both are removed when the attempt ends, including a dead job directory replaced on retry.
- Progress parsed from the remote file is in-memory only. `--out-format=%i` does not include names. Do not copy `progress` or `err` into the data dir.
- No `jobs` table and no new `settings` key.

---

## 10. Where it lands

| Crate | Change |
|-------|--------|
| `ft-exec` | Detached remote wrapper, telemetry SSH, transient-vs-fatal classification. Bump when this ships (new behavior). |
| `ft-app` | Sleep observers, retry loop, “Reconnecting…” status. Bump when this ships. |
| `ft-store` | None |
| `ft-mdns` | None |

---

## 11. Decisions

| Topic | Decision |
|-------|----------|
| Trigger | `NSWorkspace` will-sleep / did-wake, plus any transient SSH/rsync death. Not a lid-angle sensor. |
| Stay awake | No. Sleep is allowed. |
| Remote→remote | Detach at start (new session). Telemetry SSH is disposable. Exit file decides success. |
| Controller endpoint | Same rsync flags again after the child dies. No `--append-verify`. |
| Local→local | Keep the original child across wake. |
| Plan | In-memory `TransferPlan` only. No store change. |
| How long to retry | Until success, cancel, or a fatal error. After wake, wait for `ssh true` first. |
| Quit | Best-effort kill of the remote pid. No resume after relaunch. |

---

## 12. Testing (when implemented)

- Remote→remote: start a long copy, close the lid for half a minute, open it. Progress has moved, then the job completes. Sidebar never shows Failed for the sleep.
- Remote→remote: leave the lid closed until the copy would have finished. Open it. **Transfer complete**, then the five-second reset.
- Local→remote, large file: close the lid mid-file, open it. The destination eventually matches the source. The partial file may be sent again.
- Local→local: close the lid mid-copy, open it. One rsync, still running.
- Clamshell (power + external display): closing the lid does not drop progress and does not start a second rsync.
- Cancel after reopen: remote `rsync` is gone.
- Quit while a remote job runs and SSH is up: remote `rsync` is gone. Quit while offline: it may finish; relaunch does not attach.
- SQLite still has no filenames and no job rows.
