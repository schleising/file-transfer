# Design: Hard link when source and destination are on the same volume

**Status:** Not implemented. The as-built Mac app remains [DESIGN.md](DESIGN.md). This document specifies when a transfer creates **hard links** instead of writing a second copy of the bytes.

The usual case is two folders on one Raspberry Pi filesystem (the same-host job in [DESIGN.md](DESIGN.md) §7). The same rule applies to two folders on this Mac when they are on one volume.

---

## 1. Overview

A hard link is a second directory entry for an existing file. It is not a btrfs reflink and it is not `cp`. Both names share one inode: size, contents, and permissions are the same object. Changing the file through either path changes the other. Deleting one name leaves the other in place.

Today every mode, including same-host, runs rsync with `--inplace` and writes new data. A same-volume job is that same rsync with `--link-dest` pointed at the source folder, and without `--inplace`. Any other job stays `--inplace`.

```
Same volume and, on btrfs, the same subvolume:

  rsync --link-dest=source  source/  dest/
  source/a/file  ── hard link ──►  dest/a/file
                     one inode

Different volume, different btrfs subvolume, or two machines:

  source  ════ rsync --inplace (unchanged) ════►  dest
```

---

## 2. Goals and non-goals

### Goals

1. If the source folder and the destination folder are on the same volume, and on btrfs the same subvolume, the job is still rsync. Files it creates at the destination are hard links to the source. File bytes are not written for those names.
2. The job keeps today’s rsync benefits: `--files-from` for the selection, `-a -r` for directories and attributes, `progress2` for the bar, and the same cancel path.
3. If the folders are on different volumes, or on different btrfs subvolumes, the job is the current rsync copy, including `--inplace`. The user does not pick a mode. A hard link cannot cross a subvolume boundary.
4. Access status says that the job will link, not copy, before **Transfer** is enabled.
5. Filenames stay out of SQLite and out of logs.

### Non-goals

- Copy-on-write clones (`cp --reflink`, APFS `cp -c`, btrfs send/receive). A reflink would look like an independent file. This design shares the inode.
- Hard-linking a directory. Linux and macOS reject that. Directories are created; files inside them are linked.
- Linking across two computers, even when both disks are btrfs.
- Linking across two volumes on one computer (SD card to USB, Macintosh HD to an external disk).
- Linking across two btrfs subvolumes. They often share one `st_dev`, and `link` still fails. A reflink could share extents across those subvolumes; this job copies instead.
- A separate `ln` or `cp -l` walk. rsync creates the links.
- A checkbox. Same volume means this rsync. Anything else means the existing copy.

---

## 3. Which jobs

| Mode | Same volume? | What runs |
|------|----------------|-----------|
| Local → local | yes | Homebrew rsync with `--link-dest` set to the source folder |
| Local → local | no | Homebrew rsync with `--inplace`, as today |
| Same SSH endpoint ([DESIGN.md](DESIGN.md) §7) | yes | That host’s rsync with `--link-dest`, over the controller SSH session |
| Same SSH endpoint | no | That host’s rsync with `--inplace`, as today |
| Local ↔ remote, or remote A ↔ remote B | — | rsync with `--inplace`, as today. The folders are not on one volume. |

Identical source and destination folders stay rejected.

“Same volume” starts as `stat` device identity (`st_dev`) of the two folder paths, read on the machine where both paths live. `stat -c '%d'` prints it. Equal device numbers are one mount. Comparing the SSH host is not enough: two folders on one Pi can still be different disks.

On btrfs that check is not enough. Subvolumes of one filesystem usually share `st_dev`, and a hard link between them fails. When either path reports filesystem type `btrfs` (`stat -f -c '%T'`), preflight also reads each path’s subvolume id with `btrfs inspect-internal rootid`. The link job runs only when those ids match. Different ids take the `--inplace` copy. If `rootid` is missing or fails, preflight chooses the copy, and does not report a hard link.

The check runs in preflight, after the folders are known, on the host that holds them:

- This Mac: `stat` both paths locally. APFS has no btrfs subvolumes.
- Same SSH host: one remote `stat` of both paths, and `rootid` when the filesystem is btrfs (Python or `stat`, same family of tools as remote listing). The controller does not infer the device from the mount table.

If either path cannot be statted, preflight fails closed and the job does not silently copy.

---

## 4. How a link job runs

The selection is still expanded to relative file paths. Hidden names stay skipped. The runner is the same as today: Homebrew rsync on this Mac, or rsync on the SSH host for a same-host job. The file list is still uploaded for a remote runner and deleted when the job ends.

The argv is today’s rsync, with two changes:

- Add `--link-dest` set to the absolute source folder. rsync hard-links a destination file from that folder when the destination name is absent and the file matches, which it does, because that folder is the source.
- Omit `--inplace`. Inplace writes into the existing destination inode. It cannot create a hard link, and if the destination name is already a link to the source it would change the source.

Everything else stays: `-a -r`, `--info=progress2`, `--outbuf=N`, `--out-format=%i`, `--files-from`. Directories, including empty ones, are still created by rsync. A symlink is recreated by `-a` as a symlink, not as a second name for the symlink inode.

rsync only links when it is creating the name.

- The destination name is absent: it becomes a hard link to the source file.
- The destination name is already that inode, or an unchanged file: rsync skips it. The source is not written.
- The destination name exists and differs: rsync replaces it with a normal update (a new inode), not a link. A later run sees it as unchanged and skips it. It is not converted into a link.
- The destination name is a directory where a file was selected: rsync fails that path, as it does today. The source is unchanged.

`--link-dest` is an absolute path. A relative one is interpreted from the destination folder.

---

## 5. Progress and cancel

The bar stays the `progress2` parser from [DESIGN.md](DESIGN.md) §8. Linked files still move the bar; rsync reports them even though it did not write their bytes.

The itemize parser today counts codes that start with `>f`, `<f`, or `cf`. A same-volume job must count each file rsync links as well, so the file count does not stay at zero when every name is a new link.

**Cancel** kills the controller-side child, as today. Links already created stay. A second run skips those names.

---

## 6. When linking cannot finish

Preflight is what chooses this rsync. A different subvolume is decided there, so the job never starts `--link-dest` across that boundary. If a link still fails at runtime, rsync copies that file and continues. That copy is a new inode because `--inplace` is off, so it cannot change the source.

Any other rsync error fails the job the way a copy does today. Access status shows the error. The app still launches.

---

## 7. What the user sees

Access status, while the check passes:

- Same volume and, on btrfs, the same subvolume: **OK — hard link** and the file count.
- Different subvolume, or `rootid` unavailable: today’s copy mode text. Not a hard link.
- Otherwise: today’s copy mode text.

The summary keeps the source and destination folders. It does not list filenames.

No new setting and no extra confirmation step. The status line is the warning that the destination names are the source files.

---

## 8. What does not change

- Peer SSH for two different computers.
- `--inplace` on every job that is not a same-volume link.
- Store schema, privacy rule, wizard, and preflight gating of **Transfer**.
- [SLEEP.md](SLEEP.md). This job is still rsync. Sleep still interrupts a local rsync and still drops SSH. A name already linked matches the source, so a recommenced rsync skips it. A file rsync was updating is not `--inplace`, so a recommence does not write through a link into the source.

---

## 9. Testing

- Two folders on one btrfs subvolume (`btrfs inspect-internal rootid` matches): destination names are the same inode as the source (`stat` device and inode). Editing one name changes the other. No extra blocks allocated for the file data.
- Two folders on one btrfs filesystem but different subvolumes (`st_dev` matches, root ids differ): rsync `--inplace` copy, two inodes. Access status does not say hard link.
- Two folders on one APFS volume, this Mac: same inode check.
- Same Pi, source on the SD card and destination on USB: rsync copy, two inodes.
- Two computers: rsync copy, even if each disk is btrfs.
- Destination name absent: after the job it is the source inode. `progress2` still advances.
- Destination file already the same inode, or unchanged: rsync skips it. Source contents stay as they were.
- Destination file present and different: rsync replaces it with a new inode. Source contents stay as they were.
- Destination path is a directory where a file was selected: job fails, source unchanged.
- Cancel mid-transfer: linked names remain; a second run skips them and finishes the rest.
- Empty selected directory: rsync creates the directory and links no file.
- A symlink in the selection: the destination name is a symlink, as with rsync `-a` today.
