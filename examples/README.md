# Example scripts

Run these tag-free scripts with the standalone Scripta interpreter. No Python,
Node.js, PHP, or Docker is required. Paths below assume an extracted
package or source checkout; sample input directories must already exist.

| Script | Example command | Behavior |
| --- | --- | --- |
| interfaces.scripta | `scripta examples/interfaces.scripta` | Interfaces, typed dependencies, and interchangeable notifier implementations. |
| objects-arrays.scripta | `scripta examples/objects-arrays.scripta` | Classes, instance methods, comprehensions, slicing, spread, destructuring, and strict membership. |
| hello.scripta | `scripta examples/hello.scripta` | Prints Hello, Scripta! |
| socket-echo.scripta | `scripta examples/socket-echo.scripta 9000` | One-connection TCP binary echo server using readiness polling; port 0 selects an ephemeral loopback port. |
| binary-streams.scripta | `scripta examples/binary-streams.scripta INPUT OUTPUT` | Copies binary data in bounded chunks to an exclusively created file. |
| http-stream-upload.scripta | `scripta examples/http-stream-upload.scripta URL INPUT` | Streams a file into an HTTP POST and counts response bytes without buffering the body. |
| password-hashing.scripta | `scripta examples/password-hashing.scripta` | Hashes a public demo value with Argon2id and reports verification/rehash status without printing the input or hash. |
| html-escaping.scripta | `scripta examples/html-escaping.scripta` | Generates HTML using escaped text and a quoted title attribute; optional first argument supplies the name. |
| string-interpolation.scripta | `scripta examples/string-interpolation.scripta` | Interpolates variables, fields, indexes, arithmetic, and bools; demonstrates literal escaping. |
| equality.scripta | `scripta examples/equality.scripta` | Short and long equality operators share strict type semantics. |
| variables.scripta | `scripta examples/variables.scripta` | Explicit values, concatenation and conversion. |
| loops.scripta | `scripta examples/loops.scripta` | Prints 1, 2, 3 on separate lines. |
| functions.scripta | `scripta examples/functions.scripta` | Typed function arguments/returns; greeting and addition. |
| sqlite.scripta | `scripta examples/sqlite.scripta` | Built-in SQLite: in-memory database, bound parameters, transaction, and query rows. |
| file-organizer.scripta | `scripta examples/file-organizer.scripta downloads organized` | Previews shallow file moves grouped by extension; --apply performs them, refusing collisions. |
| log-cleaner.scripta | `scripta examples/log-cleaner.scripta logs --days=30` | Previews regular .log files older than the threshold; --apply deletes only those files. |
| directory-backup.scripta | `scripta examples/directory-backup.scripta documents backup-new` | Previews recursive copying to a new destination with an existing parent, outside the source tree; --apply copies bytes without overwrite. |
| atomic-report.scripta | `scripta examples/atomic-report.scripta report.json --apply` | Previews without --apply; publishes a complete JSON report using atomic file replacement. Parent directory must exist. |
| server-health.scripta | `scripta examples/server-health.scripta` | Local JSON metrics, system RAM and interval-based CPU sample; optional first argument selects a disk path. |
| http-client.scripta | `scripta examples/http-client.scripta https://example.com/api/status` | Explicit bounded HTTPS/HTTP request and JSON output. Needs the endpoint and valid TLS. |
| [timezones.scripta](timezones.scripta) | `scripta examples/timezones.scripta [--local]` | Displays one instant in UTC and named zones using bundled offline data; `--local` also shows the current host-local time. |
| http-timing.scripta | `scripta examples/http-timing.scripta [URL]` | Local monotonic timing, or HTTP duration and failed-call elapsed time for an explicit URL. |
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
requires Git. Library helpers follow the same source rules as entry scripts,
including when loaded with `require_once`.

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

## Web applications and operations

Run the commands in this section from the repository or extracted-package root.
Web examples require `scripta serve`; ordinary CLI execution has no request context.

| Purpose | Example and guide |
| --- | --- |
| HTML, JSON, redirects, and SQLite | [web-app.scripta](web-app.scripta); [local application](../docs/web-servers.md#run-a-local-application) |
| Cookies and shared sessions | [web-sessions.scripta](web-sessions.scripta); [sessions](../docs/web-servers.md#sessions-and-cookies) |
| Multipart forms and uploads | [web-uploads.scripta](web-uploads.scripta); [multipart handling](../docs/web-servers.md#multipart-forms-and-uploads) |
| Incremental responses and flushing | [web-streaming.scripta](web-streaming.scripta); [response streaming](../docs/web-servers.md#incremental-responses-and-flushing) |
| Raw uploads and attachment downloads | [web-transfers.scripta](web-transfers.scripta); [file transfers](../docs/web-servers.md#raw-uploads-and-file-downloads) |
| Proxy trust and host allowlists | [web-proxy.scripta](web-proxy.scripta), [proxy-serve.toml](proxy-serve.toml), and [Caddyfile.proxy](Caddyfile.proxy) |
| JSON host logs and private health probes | [observe-serve.toml](observe-serve.toml); [observability](../docs/web-servers.md#logs-and-health-checks) |
| Development reloads | [watch/main.scripta](watch/main.scripta) and [watch/serve.toml](watch/serve.toml); [watch semantics](../docs/web-servers.md#development-watch-mode) |
| Service deployment and instance replacement | [deploy/](deploy/); [deployment instructions](../docs/web-servers.md#deployment-and-service-management) |

`web-app.scripta` demonstrates HTML, JSON, redirects, JSON POST bodies, and a SQLite
counter. Run it with `scripta serve examples/web-app.scripta`; it requires a web
request context rather than ordinary CLI execution. The SQLite counter creates
`web-demo.sqlite` in the working directory. See [web hosting](../docs/web-servers.md).

`directory-links.scripta DIRECTORY [--follow-links]` lists entry snapshots and
optionally resolves links under the runtime's filesystem read/root policies.

`web-sessions.scripta` demonstrates session visit counts, ID rotation, logout,
preference cookies, and early session close. For local HTTP, run
`scripta serve examples/web-sessions.scripta --session-cookie-secure false`.
Keep the Secure session-cookie default for public HTTPS deployments.

`serve.toml` is an optional server configuration example. Run
`scripta serve examples/web-app.scripta --config examples/serve.toml` from the
repository root. Add `--check-config` to validate without listening or
`--print-config` to inspect redacted effective settings. CLI options override the
file. See [server configuration](../docs/web-servers.md#server-configuration).

`web-proxy.scripta`, `proxy-serve.toml`, and `Caddyfile.proxy` demonstrate resolved
client/origin metadata and redirects behind an explicitly trusted proxy. Replace
`app.example.com` in both configurations before deployment. Run
`scripta serve examples/web-proxy.scripta --config examples/proxy-serve.toml`;
see [proxy trust](../docs/web-servers.md#trusted-proxy-metadata-and-allowed-hosts)
for the companion Caddy command, allowlist rules, and forwarded-header contract.

`observe-serve.toml` enables JSON host logs and a separate loopback health listener
while serving `web-proxy.scripta`. Start with
`scripta serve examples/web-proxy.scripta --config examples/observe-serve.toml`,
then request the application on port 8080 and `/livez`, `/readyz`, or `/metrics`
on port 9090. These management paths never execute application code. See
[logs and health checks](../docs/web-servers.md#logs-and-health-checks) for privacy,
resource limits, readiness, and worker recovery.

The dedicated `watch/` application demonstrates opt-in development reloads:

```sh
scripta serve examples/watch/main.scripta --config examples/watch/serve.toml
```

Edit `examples/watch/message.inc`, wait for `reload_ready`, and refresh the application.
See [watch roots, limits, and generation semantics](../docs/web-servers.md#development-watch-mode).

The [deployment templates](deploy/) include a systemd unit, blue/green server
configurations, and a Caddy front end. They illustrate private state directories,
worker recycling, separate health ports, and an operator-controlled traffic switch
followed by bounded drain. Adapt paths, account, domain, and storage before use;
follow the [deployment instructions](../docs/web-servers.md#deployment-and-service-management)
for readiness checks, replacement order, shutdown, and platform-specific service
management. Deploy new source through instance replacement; worker recycling
retains the cached application source.
