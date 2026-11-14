# Example scripts

All scripts start with <?scripta and run with the standalone scripta interpreter. No
Python, Node.js, PHP, or Docker is required. Paths below assume an extracted
package or source checkout; sample input directories must already exist.

| Script | Example command | Behavior |
| --- | --- | --- |
| hello.scripta | `scripta examples/hello.scripta` | Prints Hello, Scripta! |
| variables.scripta | `scripta examples/variables.scripta` | Explicit values, concatenation and conversion. |
| loops.scripta | `scripta examples/loops.scripta` | Prints 1, 2, 3 on separate lines. |
| functions.scripta | `scripta examples/functions.scripta` | Typed function arguments/returns; greeting and addition. |
| sqlite.scripta | `cargo run -p scr-cli --features sqlite -- examples/sqlite.scripta` | Optional built-in SQLite: in-memory database, bound parameters, transaction, and query rows. |
| file-organizer.scripta | `scripta examples/file-organizer.scripta downloads organized` | Previews shallow file moves grouped by extension; --apply performs them, refusing collisions. |
| log-cleaner.scripta | `scripta examples/log-cleaner.scripta logs --days=30` | Previews regular .log files older than the threshold; --apply deletes only those files. |
| directory-backup.scripta | `scripta examples/directory-backup.scripta documents backup-new` | Previews recursive copying to a new destination with an existing parent, outside the source tree; --apply copies bytes without overwrite. |
| atomic-report.scripta | `scripta examples/atomic-report.scripta report.json --apply` | Previews without --apply; publishes a complete JSON report using atomic file replacement. Parent directory must exist. |
| server-health.scripta | `scripta examples/server-health.scripta` | Local JSON metrics, system RAM and interval-based CPU sample; optional first argument selects a disk path. |
| http-client.scripta | `scripta examples/http-client.scripta https://example.com/api/status` | Explicit bounded HTTPS/HTTP request and JSON output. Needs the endpoint and valid TLS. |
| data-handling.scripta | `scripta examples/data-handling.scripta` | Local byte conversions, emoji-aware slicing, CSV round trip, and simple regex. |
| csv-reader.scripta | `scripta examples/csv-reader.scripta records.csv` | Streams complete CSV records as JSON lines, including quoted multiline fields. |
| download.scripta | `scripta examples/download.scripta https://example.com/archive.zip archive.zip` | Downloads exact bytes into a new file; default HTTP ceiling is 8 MiB. Requires explicit URL and destination. |
| command-runner.scripta | `scripta examples/command-runner.scripta git '["status","--short"]'` | Explicit executable plus JSON array of literal arguments, bounded capture/timeout; needs the chosen executable. |

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

Additional existing examples include reuse.scripta with lib/helpers.scripta, system.scripta,
process.scripta, and organize.scripta. The latter legacy organizer moves PDFs when run,
so prefer file-organizer.scripta for a preview-first workflow. process.scripta explicitly
requires Git. Library helpers require <?scripta even when loaded with require_once.

The `packages/` example installs a local versioned library and loads it by name:

```sh
cd examples/packages
scripta install --locked --offline
scripta main.scripta
```

It prints `Hello, packages!`. Its manifest and lockfile are included; installed
files go in the ignored `.scripta/` directory. See [package management](../docs/packages.md)
for Git sources, version constraints, and updates.

The cross-platform release_examples integration suite uses temporary directories,
loopback HTTP, and the scripta executable itself for child-process testing. It does
not invoke public endpoints, Git, Python, Node.js, or PHP.
