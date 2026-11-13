# Example scripts

All scripts start with <?scr and run with the standalone scr interpreter. No
Python, Node.js, PHP, or Docker is required. Paths below assume an extracted
package or source checkout; sample input directories must already exist.

| Script | Example command | Behavior |
| --- | --- | --- |
| hello.scr | `scr examples/hello.scr` | Prints Hello, Scripta! |
| variables.scr | `scr examples/variables.scr` | Explicit values, concatenation and conversion. |
| loops.scr | `scr examples/loops.scr` | Prints 1, 2, 3 on separate lines. |
| functions.scr | `scr examples/functions.scr` | Typed function arguments/returns; greeting and addition. |
| sqlite.scr | `cargo run -p scr-cli --features sqlite -- examples/sqlite.scr` | Optional built-in SQLite: in-memory database, bound parameters, transaction, and query rows. |
| file-organizer.scr | `scr examples/file-organizer.scr downloads organized` | Previews shallow file moves grouped by extension; --apply performs them, refusing collisions. |
| log-cleaner.scr | `scr examples/log-cleaner.scr logs --days=30` | Previews regular .log files older than the threshold; --apply deletes only those files. |
| directory-backup.scr | `scr examples/directory-backup.scr documents backup-new` | Previews recursive copying to a new destination with an existing parent, outside the source tree; --apply copies bytes without overwrite. |
| server-health.scr | `scr examples/server-health.scr` | Local JSON metrics, system RAM and interval-based CPU sample; optional first argument selects a disk path. |
| http-client.scr | `scr examples/http-client.scr https://example.com/api/status` | Explicit bounded HTTPS/HTTP request and JSON output. Needs the endpoint and valid TLS. |
| command-runner.scr | `scr examples/command-runner.scr git '["status","--short"]'` | Explicit executable plus JSON array of literal arguments, bounded capture/timeout; needs the chosen executable. |

PowerShell also accepts the single-quoted JSON argument shown above. A program or
file path containing spaces should be quoted as one argument. Destructive/moving
examples default to preview; add --apply only after reviewing paths/output. Backup
copies do not preserve timestamps/permissions, skip symlinks, are depth-limited,
and leave partial new output after failure; originals are not deleted. Existing
backup destinations are refused. These are examples rather than a backup product.

HTTP is never requested merely by loading an example; http-client requires a URL
or exits with usage. Metrics can raise UnsupportedError on a restricted host.
The command runner does not interpret pipes, substitutions, or shell syntax;
JSON arguments are passed directly to Process::run.

Additional existing examples include reuse.scr with lib/helpers.scr, system.scr,
process.scr, and organize.scr. The latter legacy organizer moves PDFs when run,
so prefer file-organizer.scr for a preview-first workflow. process.scr explicitly
requires Git. Library helpers require <?scr even when loaded with require_once.

The `packages/` example installs a local versioned library and loads it by name:

```sh
cd examples/packages
scr install --locked --offline
scr main.scr
```

It prints `Hello, packages!`. Its manifest and lockfile are included; installed
files go in the ignored `.scripta/` directory. See [package management](../docs/packages.md)
for Git sources, version constraints, and updates.

The cross-platform release_examples integration suite uses temporary directories,
loopback HTTP, and the scr executable itself for child-process testing. It does
not invoke public endpoints, Git, Python, Node.js, or PHP.
