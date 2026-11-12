---
title: Browsing
description: Open a directory in dfdu, the keys, the views, mounts and the scan cache.
---

```sh
dfdu                      # browse the current directory
dfdu ~                    # browse a directory
dfdu ~/dev ~/Downloads    # browse multiple directories
dfdu --disks              # pick a disk first
dfdu --type video ~       # only video files
dfdu --remote server /srv # another host over ssh (dfdu installed there)
```

Multiple paths start independent local scans in the interactive terminal browser. Deleting items is disabled for multi-root scans; scan one root at a time to delete. Native cleanup commands remain available. Use `Ctrl-G` to go to a folder in any of the scanned roots. Multiple paths are not supported with `--remote` or when stdout is not a terminal. The `--disks`, `--file` and `--resume-from` options cannot be combined with paths. Subcommands such as `dfdu scan` accept one path, not multiple paths. On Windows, `--mft` is not used for multiple roots; dfdu lists directories instead.

## Keys

Keys follow ncdu. Press `?` in the app for the full list.

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| `↑↓←→` / `hjkl` | Move, open, go back | `1` `2` | List, treemap |
| `s` `n` `C` `M` | Sort by size, name, count, date | `3` | Cleanup |
| `T` | Tree mode | `4` | Duplicates |
| `/` `t` | Filter by name, by file type | `5` | Largest files |
| `v` | Mark a range | `d` | Move to Trash |
| `o` `y` | Open in file manager, copy path | `D` | Delete permanently |
| `r` | Rescan | `q` | Quit |
| `Ctrl-G` | Go to a folder | `?` | All keys |
| `B` | Bind or unbind (peer mode) | | |
| `W` | Swap peer roles (master only) | | |

Unreadable entries are marked `!` with the reason. Press `!` to list them all.

## Peer mode (Unix)

Start two independent browsers with `dfdu PATH`. Peer mode is on by default for local
interactive browsers on a terminal. Use `--no-peer` to opt out; `--peer` remains accepted.
Subcommands and piped output do not start peer mode. Each browser gets a short random ID.
While unbound, the browser discovers hosts at startup and about every 10 seconds.
The footer shows `◆ HOST found · B to connect`, or a count when several hosts answer.
Hosts disappear after three unanswered rounds. Binding stops these background probes
and hides the notice; unbinding starts discovery again.
Press `B` to open the peers panel with discovered hosts already selected while it refreshes.
Enter binds directly when the host advertises one instance. With several instances,
Enter fetches the list through SSH; select an instance and press Enter again.
Local instances bind without SSH. The browser where you press Enter becomes master.
Press `r` to refresh or `m` for manual `ID`, `HOST ID` or `HOST:ID` entry.
Press `B` again on either bound side to unbind; Escape closes the panel or prompt. The footer shows the local ID when unbound, or the role and
partner's host/ID when bound. A second binder is refused. Connection errors appear in the
status line. Browsing remains independent: navigation sync and role swapping
are not implemented yet. `--bind TARGET` binds at startup.

```sh
dfdu peers                    # list live instances on this host
dfdu peers --json             # metadata for scripts
dfdu peers --discover         # LAN and online tailnet hosts
dfdu --bind ab12cd34 /backup
dfdu --bind 'nas ab12cd34' /source
dfdu --peer-remote-bin /opt/bin/dfdu /source
make install                  # copy the release binary to ~/.local/bin/dfdu
```

Local binding opens the instance's Unix socket directly. Remote binding runs
`ssh -T -o BatchMode=yes HOST ~/.local/bin/dfdu peer-attach ID`. Install this fork on both
hosts. The fixed binary path avoids an unrelated upstream dfdu on the remote PATH.
`--peer-remote-bin` overrides it; the remote shell expands a leading `~/`.
`DFDU_SSH` can select another ssh executable, mainly for testing. Authentication and
host-key checks use your normal ssh configuration. There is no TCP listener.

Discovery uses UDP port 47839 (`DFDU_DISCOVERY_PORT` overrides it). Beacons contain only
host, user and instance IDs. They are hints, not authentication. Probes use LAN broadcast
and online IPv4 tailnet peers from `tailscale status --json`. Missing tailscale is silent;
`DFDU_TAILSCALE` overrides its executable. Details and binding always use SSH.

Registrations live in `~/.local/state/dfdu/peers/`, independent of the temporary and XDG
runtime directories. The directory is owner-only (`0700`), sockets and JSON metadata are
`0600`. Metadata contains ID, hostname, PID, roots, start time (Unix seconds) and role.
Normal exit, panic and termination signals clean registrations; `dfdu peers` also removes
stale entries. SIGKILL cannot run cleanup. A socket path longer than 103 bytes is refused
with a clear error. On macOS a refused endpoint with a still-live PID is retained because a
full listener backlog can also refuse connections.

`dfdu peer-attach ID` bridges standard input/output to the local socket and stops when
either side closes. It is the remote ssh entry point, not an interactive browser.
The stream uses bounded newline-delimited JSON, a version-1 hello, heartbeats and bye.
Connections time out after ten seconds without a valid frame. No paths or navigation
requests are sent in this phase.

## Views

![The cleanup view](/demos/cleanup.gif)

The cleanup view (`3`) lists what the [cleanup rules](/docs/rules/) found, with how safe each match is to delete and how much it frees. The treemap (`2`), tree mode (`T`), duplicates (`4`) and largest files (`5`) are one key away from the list.

## Image previews

In kitty, highlighting a JPEG, PNG or ARW file in the Files view (`1`, including tree mode) shows a preview beside its row. No key or option is needed. The preview starts one cell after the selected row's displayed name and notes, at the same row. It uses 30 columns by 12 rows, clipped to the screen. It does not cover the selected name; if no space remains beside it, no preview is shown.

Previews read local files only, not `--remote` scans or imported snapshots. They are enabled when `TERM` contains `kitty` or `ghostty`, `TERM_PROGRAM` is `ghostty`, or `KITTY_WINDOW_ID` is set. Set `DFDU_IMAGE_PREVIEW=0` to disable them, or `DFDU_IMAGE_PREVIEW=1` to force them on, including inside multiplexers. Inherited terminal variables also count inside multiplexers, which must pass kitty graphics through for the image to display. Decoding runs in the background; moving before it finishes discards the old result. An unreadable or invalid image has no preview.

Changing the selection or view, opening an overlay or the debug panel, leaving the browser, or handing the terminal back for a connection clears the image. Closing the overlay or returning to Files restores it.

## Other folders

The scan starts at the folder you named, but you can leave it. At the top of the scan, `←` says so; press it again within a second or so to scan the folder above. `Ctrl-G` opens a prompt with the current folder: type a path (absolute, or relative to the current folder) and press Enter. On a scan of this computer, a path can also start with `~` for your home folder. A folder inside the scan opens at once; one outside it is scanned, with the same options. A folder inside the scan that the scan has not reached, or a mount it did not enter, keeps you where you are, and the status line says why. `Tab` completes a folder name, and Esc closes the prompt.

Clear the prompt, or leave only `/` (or a drive such as `C:` on Windows), and it lists the host's mounts with their filesystem type and how much of each is used. `↑` and `↓` pick one, and `Tab` or Enter puts it in the prompt.

After going up, the folder you came from is selected in the new scan once it appears. When the scan's folder is empty, the list says so and how to go up (`←` twice) or elsewhere (`Ctrl-G`).

The new scan starts before the old one stops, so if it cannot start you stay where you were and the status line says why. Going up from `/mnt/cache` scans `/mnt` on its own filesystem only, so the other disks mounted there are listed but not entered; press `m` on one to scan it.

On this computer, a scan of a folder above the old one keeps what the old scan read. Only the rest of the folder is listed. Each folder already read is checked by its modification time and read again if it changed, as in a scan opened from the cache. A file edited in place inside a folder that did not change can show its old size until you rescan (`r`). The old scan can still be running: the folders it had not finished are read again. It must not have ended incomplete, and nothing can be being deleted or waiting to be undone. When the old folder is a disk or volume of its own, the new scan goes into it and keeps it, even when it does not cross into other filesystems. Otherwise, or when the folder's scan is in the cache, or another disk or volume lies between the two folders, the folder is scanned as usual.

On a scan of another host (`--remote`), going up or to a folder outside the scan connects again with the new folder. dfdu gives the terminal back while it connects, so ssh can ask for a password. `Tab` asks the host for its folders and shows them when they arrive, and the mount list is the host's. The prompt does not wait for the answer: you can keep typing. A scan read from a file (`dfdu --import`) cannot leave the file: run `dfdu` with a folder instead.

## Mounts

Like `ncdu -x`, a scan stays on one filesystem. Add `--cross-mounts` for other local filesystems and `--network` for network mounts.

## The scan cache

Single-root scans that took 3 s or more are cached, so the next `dfdu PATH` opens at once and refreshes in the background (`cached 3 h ago, refreshing` in the header). On macOS, Windows and btrfs only the changed directories are read again. Multi-root scans do not read or write the scan cache and are not recorded in scan history.

A cache is used for 14 days and the cache keeps at most 1 GiB, readable only by you. `--no-cache` scans afresh, `DFDU_CACHE=0` turns it off, `dfdu cache list` and `dfdu cache clear` manage it.

## Deleting

Deletes go to the Trash. Permanent deletes (`D`) ask you to type `delete`. `--read-only` removes the delete keys.
