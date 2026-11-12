
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'dfdu' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'dfdu'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'dfdu' {
            [CompletionResult]::new('--bind', '--bind', [CompletionResultType]::ParameterName, 'Bind to ID, ''HOST ID'' or HOST:ID at startup')
            [CompletionResult]::new('--peer-remote-bin', '--peer-remote-bin', [CompletionResultType]::ParameterName, 'Remote dfdu executable for peer binding (expanded by the remote shell)')
            [CompletionResult]::new('--resume-from', '--resume-from', [CompletionResultType]::ParameterName, 'Open the scan saved in FILE (a snapshot or an ncdu export) at once, and rescan its directory in the background, replacing it when done. A snapshot is rescanned with the mount options it was taken with')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Browse a scan saved in FILE (a snapshot, an ncdu JSON export, plain or zstd, or an ncdu binary export from `ncdu -O`; `-` for stdin), read-only')
            [CompletionResult]::new('--file', '--file', [CompletionResultType]::ParameterName, 'Browse a scan saved in FILE (a snapshot, an ncdu JSON export, plain or zstd, or an ncdu binary export from `ncdu -O`; `-` for stdin), read-only')
            [CompletionResult]::new('--remote', '--remote', [CompletionResultType]::ParameterName, 'Browse PATH on HOST, scanned there by the dfdu agent through ssh (sent there if HOST has none, see --remote-agent) and classified with the rules of this host. The delete keys delete on HOST, unless --read-only; nothing is recorded in the history')
            [CompletionResult]::new('--remote-agent', '--remote-agent', [CompletionResultType]::ParameterName, 'How to start the agent on HOST: auto (an installed dfdu that speaks this one''s protocol, else one sent earlier, else send this version''s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]')
            [CompletionResult]::new('--type', '--type', [CompletionResultType]::ParameterName, 'Show only files of these types or extensions, comma-separated: `video,audio,.iso` (`t` in the browser changes it)')
            [CompletionResult]::new('--theme', '--theme', [CompletionResultType]::ParameterName, 'Colour theme: auto, dark, light, ansi (the terminal''s own 16 colours), mono, high-contrast, or NAME for the theme file NAME.toml in dfdu/themes in the configuration directory. auto asks the terminal for its background colour at start (up to 1 s if it does not answer) and picks dark or light, or ansi when the colour is unknown')
            [CompletionResult]::new('--color', '--color', [CompletionResultType]::ParameterName, 'Colours: auto (from the terminal and NO_COLOR), never, 16, 256 or truecolor. A level given here is used even when NO_COLOR is set')
            [CompletionResult]::new('--bars', '--bars', [CompletionResultType]::ParameterName, 'Bars: unicode (eighth blocks, the default) or ascii (# and =, for terminals and fonts without block characters). Overrides the theme file''s `bars`')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--no-rules', '--no-rules', [CompletionResultType]::ParameterName, 'Do not classify entries: no categories, no cleanup candidates')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('--disks', '--disks', [CompletionResultType]::ParameterName, 'Choose a disk to browse from the mounted filesystems')
            [CompletionResult]::new('--read-only', '--read-only', [CompletionResultType]::ParameterName, 'Never delete anything: the browser leaves out its delete keys and runs cleanup commands only as dry runs')
            [CompletionResult]::new('--no-history', '--no-history', [CompletionResultType]::ParameterName, 'Do not record the scan in the history, nor show what grew since the last one')
            [CompletionResult]::new('--no-cache', '--no-cache', [CompletionResultType]::ParameterName, 'Scan PATH afresh: do not open its cached scan, nor save this one (`DFDU_CACHE=0` turns the cache off for good). Without it, the browser opens the last complete scan of PATH at once, if it is at most 1 hour old (`DFDU_CACHE_MAX_AGE`, e.g. 20m, 1h or 2d), and brings it up to date in the background; scans that take 3 s or more are kept for 14 days (`dfdu cache list`)')
            [CompletionResult]::new('--no-mouse', '--no-mouse', [CompletionResultType]::ParameterName, 'Do not capture the mouse (click to select, double-click to open, wheel to scroll). While it is captured, most terminals select text with Shift held down')
            [CompletionResult]::new('--debug-stats', '--debug-stats', [CompletionResultType]::ParameterName, 'Start with the debug panel open, and print its numbers when the browser exits')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('peers', 'peers', [CompletionResultType]::ParameterValue, 'List live peer-mode instances on this host, removing stale entries')
            [CompletionResult]::new('peer-attach', 'peer-attach', [CompletionResultType]::ParameterValue, 'Bridge stdin and stdout to a local peer instance (used over ssh)')
            [CompletionResult]::new('scan', 'scan', [CompletionResultType]::ParameterValue, 'Scan a directory and print a summary')
            [CompletionResult]::new('disks', 'disks', [CompletionResultType]::ParameterValue, 'List the mounted filesystems with their size and free space')
            [CompletionResult]::new('report', 'report', [CompletionResultType]::ParameterValue, 'Scan a directory and list what the cleanup rules found in it: caches, build output, dependencies, with how safe each is to delete')
            [CompletionResult]::new('clean', 'clean', [CompletionResultType]::ParameterValue, 'Delete what `report` recommends (dry run unless --yes)')
            [CompletionResult]::new('diff', 'diff', [CompletionResultType]::ParameterValue, 'What grew: a directory now against its last recorded scan (or one from --since ago), or two saved scans')
            [CompletionResult]::new('history', 'history', [CompletionResultType]::ParameterValue, 'List the recorded scans')
            [CompletionResult]::new('dupes', 'dupes', [CompletionResultType]::ParameterValue, 'Scan a directory and list the files with identical contents (size, then partial hash, then SHA-256), the most space to free first')
            [CompletionResult]::new('export', 'export', [CompletionResultType]::ParameterValue, 'Scan a directory and write it in ncdu''s JSON format')
            [CompletionResult]::new('import', 'import', [CompletionResultType]::ParameterValue, 'Read a scan in ncdu''s JSON format and print a summary (`dfdu -f FILE` browses it)')
            [CompletionResult]::new('recover', 'recover', [CompletionResultType]::ParameterValue, 'List what earlier sessions left unfinished (staged items, interrupted deletes), and finish, restore or dismiss them')
            [CompletionResult]::new('cache', 'cache', [CompletionResultType]::ParameterValue, 'List or clear the scans the browser cached, which it opens at once next time')
            [CompletionResult]::new('man', 'man', [CompletionResultType]::ParameterValue, 'Write man pages (dfdu.1 and one per command) into DIR, for packagers')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Check for a newer release of dfdu and install it over this one (checked against its published SHA-256). A dfdu installed with Homebrew, Nix, Scoop, winget or a system package manager is left to it: the command to update it is printed instead')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Print a shell completion script (bash, zsh, fish, elvish, powershell)')
            [CompletionResult]::new('agent', 'agent', [CompletionResultType]::ParameterValue, 'Scan for a dfdu on another host (`--remote`): reads requests on stdin and writes what it finds to stdout in dfdu''s wire format. Not for use by hand')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dfdu;peers' {
            [CompletionResult]::new('--discover', '--discover', [CompletionResultType]::ParameterName, 'Discover hosts over LAN and tailnet UDP')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, 'Print instance metadata as JSON')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;peer-attach' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;scan' {
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output: table (for people), json (one document, schema/scan.v1.json), ndjson (one record per line, for large trees) or ncdu (ncdu''s JSON export)')
            [CompletionResult]::new('--depth', '--depth', [CompletionResultType]::ParameterName, 'Levels of directories to print below the root [default: 1; every level for ndjson]')
            [CompletionResult]::new('--top', '--top', [CompletionResultType]::ParameterName, 'Entries to print per directory, largest first [default: 10; all for ndjson]')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads for the scanned filesystem. Mounts below it with their own limit (spinning disks: 2, network: 4) add that many')
            [CompletionResult]::new('--measure-workers', '--measure-workers', [CompletionResultType]::ParameterName, 'Threads measuring shared extents (btrfs, XFS; APFS clones of 1 MiB or more); 0 reports sizes as du does')
            [CompletionResult]::new('--remote', '--remote', [CompletionResultType]::ParameterName, 'Scan PATH on this host, through ssh; the dfdu agent is sent there if it has none (--remote-agent). Nothing is recorded in the history')
            [CompletionResult]::new('--remote-agent', '--remote-agent', [CompletionResultType]::ParameterName, 'How to start the agent on HOST: auto (an installed dfdu that speaks this one''s protocol, else one sent earlier, else send this version''s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]')
            [CompletionResult]::new('--type', '--type', [CompletionResultType]::ParameterName, 'Count only files of these types or extensions, comma-separated: `video,audio,.iso`. Types: video, audio, image, document, archive, disk-image, code, executable, font, data, other. Folders show the size of the matching files inside them')
            [CompletionResult]::new('--stall-abandon', '--stall-abandon', [CompletionResultType]::ParameterName, 'Abandon directories stalled this many seconds')
            [CompletionResult]::new('--stall-after', '--stall-after', [CompletionResultType]::ParameterName, 'Report a directory as stalled after one call on it took this many seconds (tests)')
            [CompletionResult]::new('--backend', '--backend', [CompletionResultType]::ParameterName, 'Filesystem access: `native` (the fastest for this system) or `portable` (std::fs, the reference the native backends are tested against)')
            [CompletionResult]::new('--apparent', '--apparent', [CompletionResultType]::ParameterName, 'Show apparent sizes instead of disk usage')
            [CompletionResult]::new('--like-du', '--like-du', [CompletionResultType]::ParameterName, 'Count data shared between files (btrfs snapshots, reflinks, APFS clones) once per file, as du does, instead of once')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--no-rules', '--no-rules', [CompletionResultType]::ParameterName, 'Do not classify entries: no categories, no cleanup candidates')
            [CompletionResult]::new('--by-type', '--by-type', [CompletionResultType]::ParameterName, 'After the tree, print disk usage by file type and the largest extensions')
            [CompletionResult]::new('--bytes', '--bytes', [CompletionResultType]::ParameterName, 'Print only the root total in bytes (for scripts and tests)')
            [CompletionResult]::new('--errors', '--errors', [CompletionResultType]::ParameterName, 'List every entry that could not be read, with the reason')
            [CompletionResult]::new('--no-history', '--no-history', [CompletionResultType]::ParameterName, 'Do not record the scan in the history (`dfdu diff`, `dfdu history`)')
            [CompletionResult]::new('--debug-stats', '--debug-stats', [CompletionResultType]::ParameterName, 'Print pipeline counters and applier timing (lock hold and wait) after the scan')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dfdu;disks' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;report' {
            [CompletionResult]::new('--min-size', '--min-size', [CompletionResultType]::ParameterName, 'Only matches at least this big (`100M`, `1.5GiB`, `500 KB`)')
            [CompletionResult]::new('--category', '--category', [CompletionResultType]::ParameterName, 'Only these categories (comma separated): an id such as `dev.build`, or its first part (`dev`) for all of them')
            [CompletionResult]::new('--older-than', '--older-than', [CompletionResultType]::ParameterName, 'Only matches unchanged for this long (`30d`, `2w`, `12h`; a bare number is days)')
            [CompletionResult]::new('--safety', '--safety', [CompletionResultType]::ParameterName, 'Only matches this safe or safer: safe, regenerable, review, manual or protected (all)')
            [CompletionResult]::new('--fail-above', '--fail-above', [CompletionResultType]::ParameterName, 'Exit with code 4 when the matches listed, protected ones left out, add up to more than this (`5GiB`), for CI')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output: table or json (schema/report.v1.json)')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;clean' {
            [CompletionResult]::new('--min-size', '--min-size', [CompletionResultType]::ParameterName, 'Only matches at least this big (`100M`, `1.5GiB`, `500 KB`)')
            [CompletionResult]::new('--category', '--category', [CompletionResultType]::ParameterName, 'Only these categories (comma separated): an id such as `dev.build`, or its first part (`dev`) for all of them')
            [CompletionResult]::new('--older-than', '--older-than', [CompletionResultType]::ParameterName, 'Only matches unchanged for this long (`30d`, `2w`, `12h`; a bare number is days)')
            [CompletionResult]::new('--safety', '--safety', [CompletionResultType]::ParameterName, 'Matches this safe or safer are considered: safe, regenerable or review. Manual and protected matches are never deleted')
            [CompletionResult]::new('--mode', '--mode', [CompletionResultType]::ParameterName, 'trash (the default; frees no space until the trash is emptied), stage (moved aside at once, then deleted; what is left after a crash is offered by `dfdu recover`) or delete (permanently)')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output: table or json (schema/clean.v1.json)')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads')
            [CompletionResult]::new('--all', '--all', [CompletionResultType]::ParameterName, 'Every match that passes the filters, not only the recommended ones (recommended: `safe` above the rule''s minimum size, `regenerable` also unchanged for the rule''s minimum age, nothing changed in the last 7 days)')
            [CompletionResult]::new('--yes', '--yes', [CompletionResultType]::ParameterName, 'Delete. Without it nothing is changed: the plan is printed')
            [CompletionResult]::new('--allow-commands', '--allow-commands', [CompletionResultType]::ParameterName, 'Run the owning tool''s cleanup command for matches cleaned up that way (`docker system prune`, `go clean -cache`). Without it they are skipped')
            [CompletionResult]::new('--finish-staged', '--finish-staged', [CompletionResultType]::ParameterName, 'First delete what earlier sessions left staged (`dfdu recover` lists it)')
            [CompletionResult]::new('--restore-staged', '--restore-staged', [CompletionResultType]::ParameterName, 'First put back what earlier sessions left staged')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;diff' {
            [CompletionResult]::new('--since', '--since', [CompletionResultType]::ParameterName, 'Compare with the recorded scan closest to this long ago (`7d`, `2w`, `12h`) instead of the last one')
            [CompletionResult]::new('--depth', '--depth', [CompletionResultType]::ParameterName, 'Levels below the root to list')
            [CompletionResult]::new('--top', '--top', [CompletionResultType]::ParameterName, 'Changes to list, largest first')
            [CompletionResult]::new('--min-change', '--min-change', [CompletionResultType]::ParameterName, 'Leave out changes smaller than this (`1M`, `500K`)')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output: table or json (schema/diff.v1.json)')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;history' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;dupes' {
            [CompletionResult]::new('--min-size', '--min-size', [CompletionResultType]::ParameterName, 'Compare only files of at least this size (`1MiB`, `100KB`)')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output: table or json (schema/dupes.v1.json)')
            [CompletionResult]::new('--top', '--top', [CompletionResultType]::ParameterName, 'Groups to print, the most space to free first [default: all]')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Threads reading files [default: 4; 1 on spinning disks]')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;export' {
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Where to write (`-` for stdout)')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Where to write (`-` for stdout)')
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads')
            [CompletionResult]::new('--resume-from', '--resume-from', [CompletionResultType]::ParameterName, 'Start from this snapshot (`--snapshot`) and rescan only the directories changed since it was taken, when the volume''s change journal can tell (macOS, Windows, btrfs as root); otherwise everything. PATH and the mount options are the snapshot''s')
            [CompletionResult]::new('--remote', '--remote', [CompletionResultType]::ParameterName, 'Scan PATH on this host, through ssh; the dfdu agent is sent there if it has none (--remote-agent)')
            [CompletionResult]::new('--remote-agent', '--remote-agent', [CompletionResultType]::ParameterName, 'How to start the agent on HOST: auto (an installed dfdu that speaks this one''s protocol, else one sent earlier, else send this version''s, asking first), installed, or upload (send one without asking) [default: auto, or DFDU_REMOTE_AGENT]')
            [CompletionResult]::new('--compress', '--compress', [CompletionResultType]::ParameterName, 'Compress with zstd (ncdu reads it with `-f`; dfdu recognises it on import)')
            [CompletionResult]::new('--snapshot', '--snapshot', [CompletionResultType]::ParameterName, 'Write dfdu''s own snapshot format instead (lossless, checked, compressed; for `dfdu -f` and `--resume-from`, not for ncdu)')
            [CompletionResult]::new('-x', '-x', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--one-file-system', '--one-file-system', [CompletionResultType]::ParameterName, 'Stay on the filesystem of the scanned directory (the default; overrides --cross-mounts and --network given before it)')
            [CompletionResult]::new('--cross-mounts', '--cross-mounts', [CompletionResultType]::ParameterName, 'Descend into other local filesystems (disks, tmpfs)')
            [CompletionResult]::new('--network', '--network', [CompletionResultType]::ParameterName, 'Descend into network filesystems (NFS, SMB, sshfs and other FUSE network mounts); with or without --cross-mounts')
            [CompletionResult]::new('--mft', '--mft', [CompletionResultType]::ParameterName, 'Windows, as administrator: read each NTFS volume''s master file table instead of listing its directories, which is much faster on large volumes. Volumes whose table cannot be read are listed as usual')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dfdu;import' {
            [CompletionResult]::new('--depth', '--depth', [CompletionResultType]::ParameterName, 'Levels of directories to print below the root')
            [CompletionResult]::new('--top', '--top', [CompletionResultType]::ParameterName, 'Entries to print per directory, largest first')
            [CompletionResult]::new('--type', '--type', [CompletionResultType]::ParameterName, 'Count only files of these types or extensions, comma-separated: `video,audio,.iso`. Types: video, audio, image, document, archive, disk-image, code, executable, font, data, other. Folders show the size of the matching files inside them')
            [CompletionResult]::new('--apparent', '--apparent', [CompletionResultType]::ParameterName, 'Show apparent sizes instead of disk usage')
            [CompletionResult]::new('--by-type', '--by-type', [CompletionResultType]::ParameterName, 'After the tree, print disk usage by file type and the largest extensions')
            [CompletionResult]::new('--bytes', '--bytes', [CompletionResultType]::ParameterName, 'Print only the root total in bytes')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;recover' {
            [CompletionResult]::new('--finish', '--finish', [CompletionResultType]::ParameterName, 'Delete what is left of these staged items (numbers from the list)')
            [CompletionResult]::new('--restore', '--restore', [CompletionResultType]::ParameterName, 'Put these staged items back at their paths (next to them as NAME.restored-N if something is there now), or take these items found in the trash back out of it')
            [CompletionResult]::new('--dismiss', '--dismiss', [CompletionResultType]::ParameterName, 'Close these interrupted items: nothing is done on disk')
            [CompletionResult]::new('--journals', '--journals', [CompletionResultType]::ParameterName, 'The journal directory (the default is dfdu''s state directory)')
            [CompletionResult]::new('--yes', '--yes', [CompletionResultType]::ParameterName, 'Finish and restore. Without it nothing is changed: what would be done is printed')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;cache' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove every cached scan')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dfdu;cache;list' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;cache;clear' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;cache;help' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove every cached scan')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dfdu;cache;help;list' {
            break
        }
        'dfdu;cache;help;clear' {
            break
        }
        'dfdu;cache;help;help' {
            break
        }
        'dfdu;man' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;update' {
            [CompletionResult]::new('--check', '--check', [CompletionResultType]::ParameterName, 'Only check whether a newer release is out; change nothing')
            [CompletionResult]::new('--force', '--force', [CompletionResultType]::ParameterName, 'Install the latest release even when this build is as new (to repair a copy)')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;completions' {
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;agent' {
            [CompletionResult]::new('--threads', '--threads', [CompletionResultType]::ParameterName, 'Worker threads')
            [CompletionResult]::new('--protocols', '--protocols', [CompletionResultType]::ParameterName, 'Print the protocol versions this agent speaks (`MIN MAX`) and exit')
            [CompletionResult]::new('--peer', '--peer', [CompletionResultType]::ParameterName, 'Enable peer mode with a private local socket (the default; B binds or unbinds)')
            [CompletionResult]::new('--no-peer', '--no-peer', [CompletionResultType]::ParameterName, 'Disable peer mode for the interactive browser')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dfdu;help' {
            [CompletionResult]::new('peers', 'peers', [CompletionResultType]::ParameterValue, 'List live peer-mode instances on this host, removing stale entries')
            [CompletionResult]::new('peer-attach', 'peer-attach', [CompletionResultType]::ParameterValue, 'Bridge stdin and stdout to a local peer instance (used over ssh)')
            [CompletionResult]::new('scan', 'scan', [CompletionResultType]::ParameterValue, 'Scan a directory and print a summary')
            [CompletionResult]::new('disks', 'disks', [CompletionResultType]::ParameterValue, 'List the mounted filesystems with their size and free space')
            [CompletionResult]::new('report', 'report', [CompletionResultType]::ParameterValue, 'Scan a directory and list what the cleanup rules found in it: caches, build output, dependencies, with how safe each is to delete')
            [CompletionResult]::new('clean', 'clean', [CompletionResultType]::ParameterValue, 'Delete what `report` recommends (dry run unless --yes)')
            [CompletionResult]::new('diff', 'diff', [CompletionResultType]::ParameterValue, 'What grew: a directory now against its last recorded scan (or one from --since ago), or two saved scans')
            [CompletionResult]::new('history', 'history', [CompletionResultType]::ParameterValue, 'List the recorded scans')
            [CompletionResult]::new('dupes', 'dupes', [CompletionResultType]::ParameterValue, 'Scan a directory and list the files with identical contents (size, then partial hash, then SHA-256), the most space to free first')
            [CompletionResult]::new('export', 'export', [CompletionResultType]::ParameterValue, 'Scan a directory and write it in ncdu''s JSON format')
            [CompletionResult]::new('import', 'import', [CompletionResultType]::ParameterValue, 'Read a scan in ncdu''s JSON format and print a summary (`dfdu -f FILE` browses it)')
            [CompletionResult]::new('recover', 'recover', [CompletionResultType]::ParameterValue, 'List what earlier sessions left unfinished (staged items, interrupted deletes), and finish, restore or dismiss them')
            [CompletionResult]::new('cache', 'cache', [CompletionResultType]::ParameterValue, 'List or clear the scans the browser cached, which it opens at once next time')
            [CompletionResult]::new('man', 'man', [CompletionResultType]::ParameterValue, 'Write man pages (dfdu.1 and one per command) into DIR, for packagers')
            [CompletionResult]::new('update', 'update', [CompletionResultType]::ParameterValue, 'Check for a newer release of dfdu and install it over this one (checked against its published SHA-256). A dfdu installed with Homebrew, Nix, Scoop, winget or a system package manager is left to it: the command to update it is printed instead')
            [CompletionResult]::new('completions', 'completions', [CompletionResultType]::ParameterValue, 'Print a shell completion script (bash, zsh, fish, elvish, powershell)')
            [CompletionResult]::new('agent', 'agent', [CompletionResultType]::ParameterValue, 'Scan for a dfdu on another host (`--remote`): reads requests on stdin and writes what it finds to stdout in dfdu''s wire format. Not for use by hand')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dfdu;help;peers' {
            break
        }
        'dfdu;help;peer-attach' {
            break
        }
        'dfdu;help;scan' {
            break
        }
        'dfdu;help;disks' {
            break
        }
        'dfdu;help;report' {
            break
        }
        'dfdu;help;clean' {
            break
        }
        'dfdu;help;diff' {
            break
        }
        'dfdu;help;history' {
            break
        }
        'dfdu;help;dupes' {
            break
        }
        'dfdu;help;export' {
            break
        }
        'dfdu;help;import' {
            break
        }
        'dfdu;help;recover' {
            break
        }
        'dfdu;help;cache' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the cached scans, newest first: when each was written, its size, its entries and its directory, and where the cache is')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove every cached scan')
            break
        }
        'dfdu;help;cache;list' {
            break
        }
        'dfdu;help;cache;clear' {
            break
        }
        'dfdu;help;man' {
            break
        }
        'dfdu;help;update' {
            break
        }
        'dfdu;help;completions' {
            break
        }
        'dfdu;help;agent' {
            break
        }
        'dfdu;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
