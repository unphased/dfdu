# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_dfdu_global_optspecs
    string join \n x/one-file-system cross-mounts network mft no-rules peer bind= peer-remote-bin= disks resume-from= f/file= read-only remote= remote-agent= type= no-history no-cache theme= color= bars= no-mouse debug-stats h/help V/version
end

function __fish_dfdu_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_dfdu_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_dfdu_using_subcommand
    set -l cmd (__fish_dfdu_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c dfdu -n "__fish_dfdu_needs_command" -l bind -d 'Bind to ID, \'HOST ID\' or HOST:ID at startup (implies --peer)' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l peer-remote-bin -d 'Remote dfdu executable for peer binding (expanded by the remote shell)' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l resume-from -d 'Open the scan saved in FILE (a snapshot or an ncdu export) at once, and rescan its directory in the background, replacing it when done. A snapshot is rescanned with the mount options it was taken with' -r -F
complete -c dfdu -n "__fish_dfdu_needs_command" -s f -l file -d 'Browse a scan saved in FILE (a snapshot, an ncdu JSON export, plain or zstd, or an ncdu binary export from `ncdu -O`; `-` for stdin), read-only' -r -F
complete -c dfdu -n "__fish_dfdu_needs_command" -l remote -d 'Browse PATH on HOST, scanned there by the dfdu agent through ssh (sent there if HOST has none, see --remote-agent) and classified with the rules of this host. The delete keys delete on HOST, unless --read-only; nothing is recorded in the history' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l remote-agent -d 'How to start the agent on HOST: auto (an installed dfdu that speaks this one\'s protocol, else one sent earlier, else send this version\'s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]' -r -f -a "auto\t'The installed dfdu if it speaks this one\'s protocol, else one sent earlier, else send one (asking first)'
installed\t'The dfdu installed on the host, as `ssh HOST dfdu agent`'
upload\t'One sent earlier, else send one, without asking'"
complete -c dfdu -n "__fish_dfdu_needs_command" -l type -d 'Show only files of these types or extensions, comma-separated: `video,audio,.iso` (`t` in the browser changes it)' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l theme -d 'Colour theme: auto, dark, light, ansi (the terminal\'s own 16 colours), mono, high-contrast, or NAME for the theme file NAME.toml in dfdu/themes in the configuration directory. auto asks the terminal for its background colour at start (up to 1 s if it does not answer) and picks dark or light, or ansi when the colour is unknown' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l color -d 'Colours: auto (from the terminal and NO_COLOR), never, 16, 256 or truecolor. A level given here is used even when NO_COLOR is set' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -l bars -d 'Bars: unicode (eighth blocks, the default) or ascii (# and =, for terminals and fonts without block characters). Overrides the theme file\'s `bars`' -r
complete -c dfdu -n "__fish_dfdu_needs_command" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_needs_command" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_needs_command" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_needs_command" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_needs_command" -l no-rules -d 'Do not classify entries: no categories, no cleanup candidates'
complete -c dfdu -n "__fish_dfdu_needs_command" -l peer -d 'Enable peer mode with a private local socket (B binds or unbinds)'
complete -c dfdu -n "__fish_dfdu_needs_command" -l disks -d 'Choose a disk to browse from the mounted filesystems'
complete -c dfdu -n "__fish_dfdu_needs_command" -l read-only -d 'Never delete anything: the browser leaves out its delete keys and runs cleanup commands only as dry runs'
complete -c dfdu -n "__fish_dfdu_needs_command" -l no-history -d 'Do not record the scan in the history, nor show what grew since the last one'
complete -c dfdu -n "__fish_dfdu_needs_command" -l no-cache -d 'Scan PATH afresh: do not open its cached scan, nor save this one (`DFDU_CACHE=0` turns the cache off for good). Without it, the browser opens the last complete scan of PATH at once, if one is cached, and brings it up to date in the background; scans that take 3 s or more are cached for 14 days (`dfdu cache list`)'
complete -c dfdu -n "__fish_dfdu_needs_command" -l no-mouse -d 'Do not capture the mouse (click to select, double-click to open, wheel to scroll). While it is captured, most terminals select text with Shift held down'
complete -c dfdu -n "__fish_dfdu_needs_command" -l debug-stats -d 'Start with the debug panel open, and print its numbers when the browser exits'
complete -c dfdu -n "__fish_dfdu_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c dfdu -n "__fish_dfdu_needs_command" -s V -l version -d 'Print version'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "peers" -d 'List live peer-mode instances on this host, removing stale entries'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "peer-attach" -d 'Bridge stdin and stdout to a local peer instance (used over ssh)'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "scan" -d 'Scan a directory and print a summary'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "disks" -d 'List the mounted filesystems with their size and free space'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "report" -d 'Scan a directory and list what the cleanup rules found in it: caches, build output, dependencies, with how safe each is to delete'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "clean" -d 'Delete what `report` recommends (dry run unless --yes)'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "diff" -d 'What grew: a directory now against its last recorded scan (or one from --since ago), or two saved scans'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "history" -d 'List the recorded scans'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "dupes" -d 'Scan a directory and list the files with identical contents (size, then partial hash, then SHA-256), the most space to free first'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "export" -d 'Scan a directory and write it in ncdu\'s JSON format'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "import" -d 'Read a scan in ncdu\'s JSON format and print a summary (`dfdu -f FILE` browses it)'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "recover" -d 'List what earlier sessions left unfinished (staged items, interrupted deletes), and finish, restore or dismiss them'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "cache" -d 'List or clear the scans the browser cached, which it opens at once next time'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "man" -d 'Write man pages (dfdu.1 and one per command) into DIR, for packagers'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "update" -d 'Check for a newer release of dfdu and install it over this one (checked against its published SHA-256). A dfdu installed with Homebrew, Nix, Scoop, winget or a system package manager is left to it: the command to update it is printed instead'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "completions" -d 'Print a shell completion script (bash, zsh, fish, elvish, powershell)'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "agent" -d 'Scan for a dfdu on another host (`--remote`): reads requests on stdin and writes what it finds to stdout in dfdu\'s wire format. Not for use by hand'
complete -c dfdu -n "__fish_dfdu_needs_command" -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dfdu -n "__fish_dfdu_using_subcommand peers" -l discover -d 'Discover hosts over LAN and tailnet UDP'
complete -c dfdu -n "__fish_dfdu_using_subcommand peers" -l json -d 'Print instance metadata as JSON'
complete -c dfdu -n "__fish_dfdu_using_subcommand peers" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand peer-attach" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l format -d 'Output: table (for people), json (one document, schema/scan.v1.json), ndjson (one record per line, for large trees) or ncdu (ncdu\'s JSON export)' -r -f -a "table\t''
json\t''
ndjson\t''
ncdu\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l depth -d 'Levels of directories to print below the root [default: 1; every level for ndjson]' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l top -d 'Entries to print per directory, largest first [default: 10; all for ndjson]' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l threads -d 'Worker threads for the scanned filesystem. Mounts below it with their own limit (spinning disks: 2, network: 4) add that many' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l measure-workers -d 'Threads measuring shared extents (btrfs, XFS; APFS clones of 1 MiB or more); 0 reports sizes as du does' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l remote -d 'Scan PATH on this host, through ssh; the dfdu agent is sent there if it has none (--remote-agent). Nothing is recorded in the history' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l remote-agent -d 'How to start the agent on HOST: auto (an installed dfdu that speaks this one\'s protocol, else one sent earlier, else send this version\'s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]' -r -f -a "auto\t'The installed dfdu if it speaks this one\'s protocol, else one sent earlier, else send one (asking first)'
installed\t'The dfdu installed on the host, as `ssh HOST dfdu agent`'
upload\t'One sent earlier, else send one, without asking'"
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l type -d 'Count only files of these types or extensions, comma-separated: `video,audio,.iso`. Types: video, audio, image, document, archive, disk-image, code, executable, font, data, other. Folders show the size of the matching files inside them' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l stall-abandon -d 'Abandon directories stalled this many seconds' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l stall-after -d 'Report a directory as stalled after one call on it took this many seconds (tests)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l backend -d 'Filesystem access: `native` (the fastest for this system) or `portable` (std::fs, the reference the native backends are tested against)' -r -f -a "native\t''
portable\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l apparent -d 'Show apparent sizes instead of disk usage'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l like-du -d 'Count data shared between files (btrfs snapshots, reflinks, APFS clones) once per file, as du does, instead of once'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l no-rules -d 'Do not classify entries: no categories, no cleanup candidates'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l by-type -d 'After the tree, print disk usage by file type and the largest extensions'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l bytes -d 'Print only the root total in bytes (for scripts and tests)'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l errors -d 'List every entry that could not be read, with the reason'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l no-history -d 'Do not record the scan in the history (`dfdu diff`, `dfdu history`)'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -l debug-stats -d 'Print pipeline counters and applier timing (lock hold and wait) after the scan'
complete -c dfdu -n "__fish_dfdu_using_subcommand scan" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c dfdu -n "__fish_dfdu_using_subcommand disks" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l min-size -d 'Only matches at least this big (`100M`, `1.5GiB`, `500 KB`)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l category -d 'Only these categories (comma separated): an id such as `dev.build`, or its first part (`dev`) for all of them' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l older-than -d 'Only matches unchanged for this long (`30d`, `2w`, `12h`; a bare number is days)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l safety -d 'Only matches this safe or safer: safe, regenerable, review, manual or protected (all)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l fail-above -d 'Exit with code 4 when the matches listed, protected ones left out, add up to more than this (`5GiB`), for CI' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l format -d 'Output: table or json (schema/report.v1.json)' -r -f -a "table\t''
json\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l threads -d 'Worker threads' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand report" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l min-size -d 'Only matches at least this big (`100M`, `1.5GiB`, `500 KB`)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l category -d 'Only these categories (comma separated): an id such as `dev.build`, or its first part (`dev`) for all of them' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l older-than -d 'Only matches unchanged for this long (`30d`, `2w`, `12h`; a bare number is days)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l safety -d 'Matches this safe or safer are considered: safe, regenerable or review. Manual and protected matches are never deleted' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l mode -d 'trash (the default; frees no space until the trash is emptied), stage (moved aside at once, then deleted; what is left after a crash is offered by `dfdu recover`) or delete (permanently)' -r -f -a "trash\t''
stage\t''
delete\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l format -d 'Output: table or json (schema/clean.v1.json)' -r -f -a "table\t''
json\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l threads -d 'Worker threads' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l all -d 'Every match that passes the filters, not only the recommended ones (recommended: `safe` above the rule\'s minimum size, `regenerable` also unchanged for the rule\'s minimum age, nothing changed in the last 7 days)'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l yes -d 'Delete. Without it nothing is changed: the plan is printed'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l allow-commands -d 'Run the owning tool\'s cleanup command for matches cleaned up that way (`docker system prune`, `go clean -cache`). Without it they are skipped'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l finish-staged -d 'First delete what earlier sessions left staged (`dfdu recover` lists it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l restore-staged -d 'First put back what earlier sessions left staged'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand clean" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l since -d 'Compare with the recorded scan closest to this long ago (`7d`, `2w`, `12h`) instead of the last one' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l depth -d 'Levels below the root to list' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l top -d 'Changes to list, largest first' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l min-change -d 'Leave out changes smaller than this (`1M`, `500K`)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l format -d 'Output: table or json (schema/diff.v1.json)' -r -f -a "table\t''
json\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l threads -d 'Worker threads' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand diff" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand history" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l min-size -d 'Compare only files of at least this size (`1MiB`, `100KB`)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l format -d 'Output: table or json (schema/dupes.v1.json)' -r -f -a "table\t''
json\t''"
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l top -d 'Groups to print, the most space to free first [default: all]' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l threads -d 'Threads reading files [default: 4; 1 on spinning disks]' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand dupes" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -s o -l output -d 'Where to write (`-` for stdout)' -r -F
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l threads -d 'Worker threads' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l resume-from -d 'Start from this snapshot (`--snapshot`) and rescan only the directories changed since it was taken, when the volume\'s change journal can tell (macOS, Windows, btrfs as root); otherwise everything. PATH and the mount options are the snapshot\'s' -r -F
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l remote -d 'Scan PATH on this host, through ssh; the dfdu agent is sent there if it has none (--remote-agent)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l remote-agent -d 'How to start the agent on HOST: auto (an installed dfdu that speaks this one\'s protocol, else one sent earlier, else send this version\'s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]' -r -f -a "auto\t'The installed dfdu if it speaks this one\'s protocol, else one sent earlier, else send one (asking first)'
installed\t'The dfdu installed on the host, as `ssh HOST dfdu agent`'
upload\t'One sent earlier, else send one, without asking'"
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l compress -d 'Compress with zstd (ncdu reads it with `-f`; dfdu recognises it on import)'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l snapshot -d 'Write dfdu\'s own snapshot format instead (lossless, checked, compressed; for `dfdu -f` and `--resume-from`, not for ncdu)'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -s x -l one-file-system -d 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l cross-mounts -d 'Descend into other local filesystems (disks, tmpfs)'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l network -d 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -l mft -d 'Windows, as administrator: read each NTFS volume\'s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual'
complete -c dfdu -n "__fish_dfdu_using_subcommand export" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l depth -d 'Levels of directories to print below the root' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l top -d 'Entries to print per directory, largest first' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l type -d 'Count only files of these types or extensions, comma-separated: `video,audio,.iso`. Types: video, audio, image, document, archive, disk-image, code, executable, font, data, other. Folders show the size of the matching files inside them' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l apparent -d 'Show apparent sizes instead of disk usage'
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l by-type -d 'After the tree, print disk usage by file type and the largest extensions'
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -l bytes -d 'Print only the root total in bytes'
complete -c dfdu -n "__fish_dfdu_using_subcommand import" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -l finish -d 'Delete what is left of these staged items (numbers from the list)' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -l restore -d 'Put these staged items back at their paths (next to them as NAME.restored-N if something is there now), or take these items found in the trash back out of it' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -l dismiss -d 'Close these interrupted items: nothing is done on disk' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -l journals -d 'The journal directory (the default is dfdu\'s state directory)' -r -F
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -l yes -d 'Finish and restore. Without it nothing is changed: what would be done is printed'
complete -c dfdu -n "__fish_dfdu_using_subcommand recover" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and not __fish_seen_subcommand_from list clear help" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and not __fish_seen_subcommand_from list clear help" -f -a "list" -d 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and not __fish_seen_subcommand_from list clear help" -f -a "clear" -d 'Remove every cached scan'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and not __fish_seen_subcommand_from list clear help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and __fish_seen_subcommand_from clear" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and __fish_seen_subcommand_from help" -f -a "list" -d 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and __fish_seen_subcommand_from help" -f -a "clear" -d 'Remove every cached scan'
complete -c dfdu -n "__fish_dfdu_using_subcommand cache; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dfdu -n "__fish_dfdu_using_subcommand man" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand update" -l check -d 'Only check whether a newer release is out; change nothing'
complete -c dfdu -n "__fish_dfdu_using_subcommand update" -l force -d 'Install the latest release even when this build is as new (to repair a copy)'
complete -c dfdu -n "__fish_dfdu_using_subcommand update" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand completions" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand agent" -l threads -d 'Worker threads' -r
complete -c dfdu -n "__fish_dfdu_using_subcommand agent" -l protocols -d 'Print the protocol versions this agent speaks (`MIN MAX`) and exit'
complete -c dfdu -n "__fish_dfdu_using_subcommand agent" -s h -l help -d 'Print help'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "peers" -d 'List live peer-mode instances on this host, removing stale entries'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "peer-attach" -d 'Bridge stdin and stdout to a local peer instance (used over ssh)'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "scan" -d 'Scan a directory and print a summary'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "disks" -d 'List the mounted filesystems with their size and free space'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "report" -d 'Scan a directory and list what the cleanup rules found in it: caches, build output, dependencies, with how safe each is to delete'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "clean" -d 'Delete what `report` recommends (dry run unless --yes)'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "diff" -d 'What grew: a directory now against its last recorded scan (or one from --since ago), or two saved scans'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "history" -d 'List the recorded scans'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "dupes" -d 'Scan a directory and list the files with identical contents (size, then partial hash, then SHA-256), the most space to free first'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "export" -d 'Scan a directory and write it in ncdu\'s JSON format'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "import" -d 'Read a scan in ncdu\'s JSON format and print a summary (`dfdu -f FILE` browses it)'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "recover" -d 'List what earlier sessions left unfinished (staged items, interrupted deletes), and finish, restore or dismiss them'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "cache" -d 'List or clear the scans the browser cached, which it opens at once next time'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "man" -d 'Write man pages (dfdu.1 and one per command) into DIR, for packagers'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "update" -d 'Check for a newer release of dfdu and install it over this one (checked against its published SHA-256). A dfdu installed with Homebrew, Nix, Scoop, winget or a system package manager is left to it: the command to update it is printed instead'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "completions" -d 'Print a shell completion script (bash, zsh, fish, elvish, powershell)'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "agent" -d 'Scan for a dfdu on another host (`--remote`): reads requests on stdin and writes what it finds to stdout in dfdu\'s wire format. Not for use by hand'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and not __fish_seen_subcommand_from peers peer-attach scan disks report clean diff history dupes export import recover cache man update completions agent help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and __fish_seen_subcommand_from cache" -f -a "list" -d 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is'
complete -c dfdu -n "__fish_dfdu_using_subcommand help; and __fish_seen_subcommand_from cache" -f -a "clear" -d 'Remove every cached scan'
