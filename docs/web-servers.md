# Serving web requests

Scripta can receive HTTP requests with `scripta serve` and execute one fixed entry
script for each request. `Http::get` and related functions remain outgoing HTTP
clients. The `Web` module is available only inside request execution; CLI scripts
continue to work as before. Entry scripts and their includes follow the same
[source rules](language-spec.md#implemented-subset) as standalone scripts.

## Run a local application

```sh
scripta serve examples/web-app.scripta --host 127.0.0.1 --port 8080
```

On Windows, the same command works with `scripta.exe` (or `scripta` on PATH).
Open `http://127.0.0.1:8080/` or request `/api/status`. The example also demonstrates
redirects, JSON POST bodies, and a SQLite counter:

```sh
curl -i http://127.0.0.1:8080/redirect
curl -H 'Content-Type: application/json' -d '{"message":"hello"}' http://127.0.0.1:8080/api/echo
curl -X POST http://127.0.0.1:8080/api/visits
```

The counter requires the default SQLite feature and creates `web-demo.sqlite` in
Scripta's working directory. Run it from a dedicated application directory if you
want to keep generated data outside the source checkout. This is a local demo,
without authentication or CSRF protection; application authors must add those
before exposing state-changing endpoints to visitors.

A minimal entry script looks like this:

```scr
$request = Web::request();
if ($request->method === "GET" && $request->path === "/api/status") {
    Web::header("Content-Type", "application/json; charset=utf-8");
    echo Json::encode(["status" => "ok"]);
} else {
    Web::status(404);
    echo "Not found";
}
```

Every URL runs the configured script. There is no URL-to-source-file mapping,
implicit static file serving, or directory listing. Keep source, databases,
credentials, and logs outside any directory your front-end server publishes.
Configure the front-end server to serve a separate public directory for assets.

## Server configuration

Pass an optional TOML file after the entry script. No file is loaded implicitly:

```sh
scripta serve examples/web-app.scripta --config examples/serve.toml
scripta serve examples/web-app.scripta --config examples/serve.toml --check-config
scripta serve examples/web-app.scripta --config examples/serve.toml --print-config
```

Explicit CLI settings override file settings, which override defaults, regardless
of where `--config` appears among the options. The entry script remains the first
argument. Duplicate CLI options, duplicate TOML keys, unknown file keys, invalid
types, and invalid effective limits are errors. File syntax/types are checked
even when a CLI setting overrides the field. Files must be UTF-8 TOML, at most
65536 bytes. No new dependency is needed for this format.

File keys are the value-taking serve option names without `--`, using the same
units and bounds. All are optional. Use TOML integers for counts/bytes/timeouts,
a boolean for `session-cookie-secure`, strings for `host`, `session-dir`, and
`upload-temp-dir`, `log-format`, and `health-address`, and arrays of strings for
`trusted-proxies` and `allowed-hosts`. For example:

```toml
host = "127.0.0.1"
port = 8080
workers = 2
queue = 32
timeout-ms = 10000
connection-timeout-ms = 60000
header-timeout-ms = 10000
drain-timeout-ms = 11000
session-cookie-secure = true
```

Relative storage paths in a file resolve beside that file. CLI storage paths and
the entry-script path resolve from the current working directory. Neither changes
the application's working directory. Empty explicit storage paths are rejected;
omit them to use private temporary stores. Missing upload directories require an
existing parent; missing session ancestors may be created at startup.

`--check-config` validates entry-script syntax, effective limits, and storage path
shape/privacy without binding sockets, starting workers, executing the script, or
creating storage. `--print-config` performs the same checks and prints effective
settings as TOML. These modes are mutually exclusive. Private storage paths are
replaced with `<configured>` or `<temporary>`; entry source, environment contents,
and session records are not printed. The informational `[runtime]` table shows
recognized effective runtime limits as decimal strings. Inspection output is not
an input configuration file: its `[runtime]` table and directory placeholders must
not be passed back as settings.

The existing `SCR_MAX_STEPS`, `SCR_MAX_VALUE_BYTES`, `SCR_MAX_SCOPES`, and
`SCR_MAX_SOURCE_BYTES` environment settings continue to configure runtime limits;
they are not server-file keys or overridden by server flags. Unset web defaults
remain one million checkpoints, 16 MiB per value, 4096 scopes, and the CLI's default
source-byte ceiling. Validation does not prove that a port is available, directories
are writable, workers can start, or application dependencies/runtime types are
valid. Startup still prepares and rechecks storage before serving requests.

## Runtime and limits

The HTTP host uses Hyper and Tokio. Persistent worker **processes** each parse the
entry script once and execute it in a fresh runtime for every request. Variables,
module state, environment snapshots, response headers, input/output, and native
resources belong to that request. Workers do not launch a new interpreter for
every request. Required/imported scripts are still loaded per request; the entry
script cache is an AST cache, not bytecode or a JIT.

The parent keeps one startup snapshot of the entry script, also used by replacement
workers. In ordinary production mode, restart the service to deploy entry-script
changes. Includes and data files are not snapshotted: use an immutable application directory and a graceful
service replacement for consistent multi-file deployments.

| Option | Default | Bounds / meaning |
| --- | --- | --- |
| `--host` | `127.0.0.1` | IP address; use a local reverse proxy for public HTTPS. |
| `--port` | `8080` | 0–65535; 0 selects an available port and prints it to stderr. |
| `--workers` | CPU count, capped at 4 | 1–64 persistent processes; one active request per worker. |
| `--watch` / `--no-watch` | disabled | Opt into local source polling/snapshots, or override a file setting to disable them. |
| `--watch-debounce-ms` | `200` | 50–5000 ms of observed stable source content before building a candidate. |
| `--worker-max-requests` | `0` | 0 disables request-count recycling; 1–1000000 completed request exchanges per worker before replacement. |
| `--queue` | `32` | 1–4096 waiting requests, including reservations while receiving bodies. |
| `--max-connections` | `128` | 1–4096 open HTTP connections. Excess connections close immediately. |
| `--max-body-bytes` | `1048576` | 0–67108864, checked for both fixed and chunked bodies. |
| `--max-response-bytes` | `8388608` | 1–67108864 application body bytes, including `echo` and `Web::write`. |
| `--timeout-ms` | `10000` | 1–3600000, covering body reception, queue wait, worker recovery and execution. |
| `--connection-timeout-ms` | `60000` | 1–3600000, total accepted connection lifetime, including response writes and idle keep-alive. |
| `--header-timeout-ms` | `10000` | 1–3600000, time allowed to receive request headers; connection lifetime can end sooner. |
| `--drain-timeout-ms` | `11000` | 1–3600000, one shared shutdown budget for draining connections and workers. |

Additional fixed limits: request URI 8192 encoded bytes, header buffering 32 KiB,
100 headers, and response headers 32 KiB/100 entries. Header-read timeout defaults
to 10 seconds and maximum connection lifetime to 60 seconds (including response
writes); both are configurable above. A longer request timeout does not extend
the connection lifetime. These independent ceilings may be configured in any
order: whichever applicable deadline arrives first ends the work. HTTP/1.1
keep-alive and streaming responses are supported within these deadlines. Request
trailers, CONNECT, upgrades, and WebSockets are not supported. Non-ASCII header
values are rejected on input; request bodies can contain arbitrary bytes.

The server reserves queue capacity before buffering bodies. Overload returns
`503`, oversized bodies `413`, malformed requests `400`, worker transport failures
`502`, and request deadlines `504`. A worker that fails or exceeds its deadline is
terminated and replaced automatically with bounded backoff. Queued requests that expire
are discarded. Already completed external side effects are not rolled back.

Worker recovery runs independently of application traffic and health probes.
Idle child exit, closed output, or unsolicited IPC invalidates its ready lease and
schedules replacement; a worker lost during a request fails that request without
replaying it. Each slot retries after 100, 200, 400, 800, 1600, 3200, then 5000 ms,
remaining at 5000 ms on repeated failure. Replacement startup has a 30-second
handshake ceiling in addition to the retry delay. A completed request exchange
(including a normal buffered application-error response), or 30 seconds of
survival observed on the next idle/dispatch event, resets that slot's backoff.
A successful handshake alone does not reset failure history. Unexpected Rust
worker-loop task termination is detected by the supervisor's accept loop within
its 100 ms polling interval and restores a slot with a separate backoff using the
same bounds; 30 seconds without another loop termination resets that history.
Initial startup still fails if any configured worker cannot initialize; retries
apply after initial startup, not to invalid configuration or initial bind errors.

Set `worker-max-requests` in TOML or `--worker-max-requests` on the CLI to recycle
workers after a bounded request count. The default `0` preserves unlimited
request reuse. Counts increase only when the worker finishes the full protocol
exchange; queue cancellations and incomplete/failed streams do not count. A
flushed stream continues until execution and the worker exchange finish, subject
to existing deadlines. Recycling then stops the idle child and schedules its
replacement without interrupting response bytes already handed to the supervisor.
The same cached entry-source snapshot is used; recycling does not deploy changed
source. Includes still follow the immutable-release rule. Persistent sessions
retain their configured store, and per-request state/spools retain their normal
cleanup. Recycling a sole worker can temporarily make readiness false and queue
requests while its replacement starts.

Readiness requires at least one known healthy worker, rather than all configured
slots. Recovery never reserves an application queue slot. Requests waiting while
capacity recovers keep their original request deadlines and admission limits;
recovery does not replay failed work or extend request timeouts. Once draining is
observed, pending recovery is cancelled (the shutdown flag is polled at most every
25 ms while waiting/starting); no further replacements or count-based recycling
are initiated. Healthy workers can continue admitted queue work until the shared
drain deadline. If lost capacity prevents completion, remaining work expires or
is interrupted at the applicable deadline. OS spawn/reaping and blocked stderr
can still delay observation; this is not a process-heap or availability guarantee.

Each request defaults to one million execution checkpoints, 16 MiB per logical
value, and 4096 scopes. `SCR_MAX_STEPS`, `SCR_MAX_VALUE_BYTES`, `SCR_MAX_SCOPES`, and
`SCR_MAX_SOURCE_BYTES` retain their CLI meanings. These are not a process heap cap.
The supervisor deadline can terminate a worker blocked in native code; script
`finally` blocks may therefore be bypassed. Interactive CLI input, process
execution/inspection, and environment mutation capabilities are disabled in web
workers. Other existing capabilities and filesystem policies still apply.
Trusted application code can access host files and environment credentials; this
is not an untrusted-code sandbox. Run the service under a dedicated OS account.

Response output is buffered before transmission unless the script calls `flush()`.
Before that first transmission, uncaught errors, nonzero script
exit, or output overflow discard application output and return a generic `500`.
Error responses may exceed a deliberately tiny application output ceiling by a
small fixed amount. Bounded host events go to stderr, with separate
header-readiness and response-body completion records. Default host events omit
request contents and error/source text; see [logs and health checks](#logs-and-health-checks)
for correlation, diagnostics, and delivery semantics. `exit 0` finishes a valid response.
HEAD suppresses the transmitted body; statuses 204, 205, and 304 also send no body.
Application code should recognize HEAD when it wants it to follow a GET route.

Ctrl+C / termination signals mark readiness false immediately. The accept loop
observes the flag within its 100 ms polling interval, closes the listener, and
notifies all existing connections. Idle keep-alive connections and connections awaiting their first complete
header close promptly. A partially received later header remains subject to the
header and drain deadlines. Active connections finish their admitted request and
close instead of accepting another request. Buffered pipelined requests whose
service has not begun are not admitted; a request racing shutdown may receive a
closing `503` or a closed connection. Requests already receiving bodies or waiting
in the queue remain eligible to finish.

One `--drain-timeout-ms` budget covers connection/body reception, queued work,
execution, streamed responses, and workers, starting when the accept loop enters
draining. Existing request, header, and connection deadlines still apply and can
end work sooner. The default drain is 11 seconds; increase it explicitly for
longer work. At expiry the host aborts remaining HTTP connections, terminates and
reaps worker children, then drops queued request spools and temporary stores.
Process reaping, filesystem cleanup, and scheduler delays add overhead beyond the
configured work allowance; this is not a hard wall-clock process-exit guarantee.
The private health listener remains available until application/worker cleanup
finishes. Repeated handled signals are idempotent: they neither force an early
exit nor restart the drain clock. Use the service manager's outer stop timeout for
forced termination if the supervisor itself becomes stuck.

Script `finally` blocks may be bypassed on forced termination. Uncommitted session
changes are discarded; sessions saved before a flush or successful request remain
saved in a configured persistent store. Request-owned spools/stages are cleaned;
explicitly saved uploads and completed filesystem/database/network side effects
cannot be rolled back. External forced kills or machine failure may leave
persistent-directory debris requiring operator cleanup. Worker stdin monitoring
also exits workers when their parent pipe closes, including during native work.

## Development watch mode

Run the dedicated [watch example](../examples/watch/main.scripta):

```sh
scripta serve examples/watch/main.scripta --config examples/watch/serve.toml
# In another terminal, edit examples/watch/message.inc, save, then refresh:
curl --fail http://127.0.0.1:8080/
curl --fail http://127.0.0.1:9090/readyz
```

Use bare `--watch` or TOML `watch = true` to opt in; `--no-watch` overrides a
file-configured watcher. Normal serving starts no polling task and retains its
explicit production deployment behavior. Check/print configuration modes do not
start a watcher or create snapshots. The canonical entry's parent is the single
watch root; use a dedicated small application directory. There is no implicit
repository-wide watch or external-root discovery. Configuration files, environment
changes, package manifests, and service settings require an explicit restart.

The host polls source contents every 100 ms. Visible `.scripta`, `.scta`, `.scr`,
and `.inc` files are included recursively; comparisons use bytes, so a same-size
or timestamp-preserving edit is detected. Hidden names, `target`, and `node_modules`
subtrees are excluded, and symlink files/directories are skipped. Other files are
not copied. The existing entry argument is canonicalized first; a symlink entry
therefore chooses its target directory as the root. New visible source files and
deletions are detected. Replacing files with editor rename saves works on the
next successful scan.

Watch scans are bounded to 4096 encountered directory entries (including excluded
names at visited levels), directory depth 32, 1 MiB of relative path bytes, and
16 MiB of selected source contents per scan. Known sizes are checked before
reading, then reads remain bounded against growth. Existing per-source/runtime
limits still apply. Polling retains bounded current/previous contents and one
latest candidate notification, with at most an active, staged, and newer pending
snapshot on disk. These are source budgets, not a process heap cap. The source
root is trusted developer input; skipping symlinks is not a defense against
hostile concurrent filesystem replacement or a hung filesystem.

A content change restarts the debounce interval (`watch-debounce-ms`, default
200 ms, bounds 50–5000), and an unchanged later scan must observe that interval
before validation. This coalesces rapid saves but does not make arbitrary multi-file
edits transactional. If edits span a quiet interval, an intermediate valid
combination can become a generation. Coordinate larger edits outside the watched
root and replace them together, or stop the server while editing.

Each validated candidate is copied into a private temporary source tree. Its
workers use that tree for both entry and includes, including includes first
executed after an editor save. Includes must resolve within that snapshot;
absolute paths into the original root, parent traversal outside it, symlinked
sources, externally installed packages, and non-source includes are unavailable.
Vendor required source under the root with relative includes for this workflow.
Host-provided script/source paths refer to the snapshot. Ordinary relative
file I/O keeps the process working directory and is not snapshotted; keep data,
uploads, sessions, databases, and application writes outside source snapshots.
Application code must not modify its snapshot files. For full filesystem or
package semantics, use ordinary serving with immutable releases.

Before changing admission, the host starts a candidate worker pool with a shared
30-second startup allowance. Failure retains the old generation. Successful
staging marks readiness false, closes old keep-alive connections, and drains old
admitted requests/streams under `drain-timeout-ms`; requests cannot mix entry and
include generations. The existing listener stays bound, but the accept loop
pauses during candidate startup/drain. New TCP connections may wait in the OS
backlog; this is a development restart with possible closed connections or
truncated streams at deadline, not a zero-downtime promise. At most two configured
worker pools coexist during staging/drain. The same host session store survives
reload; completed side effects still cannot be rolled back.

`watch_ready` and `reload_ready` report the generation and local snapshot root;
`reload_draining` marks the pause. `reload_failed` reports scan, syntax, or worker
startup failure locally. Reload syntax diagnostics contain a relative source path,
byte offset, and parser code where available, without source excerpts; run
`scripta check` on that file for the full masked diagnostic. Initial entry errors
with explicit watch mode show the normal local masked diagnostic with line/column.
HTTP clients retain generic error bodies; no source diagnostics endpoint exists.
These extra local path diagnostics are specific to opt-in watch mode.

An invalid edit leaves the last good snapshot running. A failed startup also
retains the old pool; repair storage/privacy/source-limit problems before probing
it, since those failures can independently affect serving. Another source-content
change retries reload; unchanged saves/timestamps or restored non-source settings
alone do not. Scan failures log once per continuous failure and resume checking
when scans succeed. Termination cancels pending polling and candidate startup,
drains admitted work within the existing deadline, terminates staged children,
and removes owned snapshots/spools. Blocking filesystem work, OS reaping, and log
sinks can add shutdown overhead. Native platform validation remains separate.

## Deployment and service management

Reviewable examples are in [`examples/deploy`](../examples/deploy/): a Linux
[systemd template](../examples/deploy/scripta@.service), paired
[blue](../examples/deploy/blue.toml)/[green](../examples/deploy/green.toml)
configurations, and a [Caddyfile](../examples/deploy/Caddyfile). These are templates;
creating them does not install a service. Install a verified native `scripta`
binary at `/usr/local/bin/scripta`, create an unprivileged `scripta` account/group,
and provision immutable application trees at `/srv/scripta/releases/blue` and
`/srv/scripta/releases/green`, each containing `main.scripta` and its includes.
Use unique release names in subsequent deployments rather than modifying a tree
still used by workers. The account needs read/traverse access to code and config;
code, includes, and configuration should be owned by the deployment administrator
and unwritable by the service account. Set the working directory to that exact
release tree so relative includes cannot follow a changing `current` symlink.

Install reviewed configs as `/etc/scripta/blue.toml` and `green.toml`, readable by
the service account. Replace the example domain in both allowlists and Caddy.
The service template creates private state directories under `/var/lib/scripta`,
with a shared session directory and separate per-instance upload directories.
Keep databases and other application writes outside the immutable release tree,
use explicit absolute paths, and provision only the writable directories the
application requires. The template's filesystem protection permits state writes
and private temporary files. Audit environment/credential delivery separately;
never put secrets in command arguments, application source, or public logs.

The template uses foreground execution with a 3-second failure restart delay and
a five-starts-per-minute limit. Its 15-second outer stop timeout exceeds the
11-second application drain. Adjust both together. `KillMode=mixed` sends the
initial termination signal to the supervisor and reserves forced group termination
for the outer timeout, allowing isolated workers to finish. `Type=simple` does not
establish application readiness: always probe the private listener. See the
[systemd service reference](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html)
and [kill policy reference](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html).

After account, files, and Caddy have been provisioned by an operator, this Linux
sequence starts a replacement, verifies it, switches traffic, and drains the old
instance. Run privileged commands through the host's normal administration flow:

```sh
systemd-analyze verify /etc/systemd/system/scripta@.service
systemctl daemon-reload
systemctl enable --now scripta@blue
curl --fail --max-time 2 http://127.0.0.1:9090/readyz
systemctl start scripta@green
curl --fail --max-time 2 http://127.0.0.1:9091/readyz
curl --fail --max-time 5 -H 'Host: app.example.com' http://127.0.0.1:8081/
# Edit /etc/caddy/Caddyfile: change only the upstream 8080 to 8081.
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
# Verify real application traffic through your HTTPS front end before stopping blue.
curl --fail --max-time 5 https://app.example.com/
systemctl stop scripta@blue
systemctl disable scripta@blue
systemctl enable scripta@green
journalctl -u scripta@blue -u scripta@green
```

Use bounded polling for readiness if startup has not finished, and abort the
switch on any failed probe. The application smoke route must be harmless and
appropriate for your application. The management port is private and must not be
proxied publicly. The Caddy example intentionally has no health route on the
application port; management and application routes are separate. Consult
[Caddy reload](https://caddyserver.com/docs/command-line#caddy-reload) and
[reverse proxy streaming](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#streaming)
when choosing proxy timeouts and stream behavior. Validate proxy configuration
with the installed version before use.

A reload/switch is not a zero-downtime guarantee. Existing proxy connections and
streams can outlive a configuration reload or be interrupted by proxy policy;
the old backend refuses further requests after shutdown begins. Clients may see
closed connections, `503`, or truncated bodies when a deadline expires. Do not
blindly retry non-idempotent requests: a failed HTTP response does not prove that
side effects failed. Keep enough CPU/memory/worker capacity for both instances.
Shared sessions require the same local store, account, limits, and compatible
session schema; this is not distributed multi-host session storage. SQLite and
other shared data need compatible schemas and independent migration/backup plans.
Before switching, rollback means stopping green and retaining blue. After
switching, switch back only if old code remains compatible with persisted changes;
a drain does not undo writes. Keep old release files until its processes exit.

On Apple Silicon macOS, use a reviewed launchd job or a foreground supervisor with
absolute paths, a dedicated account, private state directories, and a stop grace
period longer than the application drain; the Linux unit and `/var/lib` paths are
not macOS service configuration. Windows `scripta serve` is a console process,
not a Windows Service Control Manager implementation. A service wrapper must
forward a handled console stop event and allow the drain before forced process-tree
termination; a wrapper that only kills the process cannot provide graceful drain.
Apply private account ACLs on Windows instead of relying on Unix modes. No native
Linux service manager, launchd job, or Windows wrapper was executed for these
examples; platform service validation remains pending.

## Logs and health checks

Text host logs remain the default. Set `--log-format json` (or
`log-format = "json"` in TOML) for JSON lines on stderr. Events include server
startup/drain/stop, worker startup/failure, request diagnostics, connection errors,
response headers, and response completion. JSON events include a numeric Unix
`timestampMs`; response records include `durationMs` since request dispatch.
Invalid configuration/startup errors from the CLI can still use its ordinary text
error format. `serve` syntax failures omit source excerpts; use `scripta check`
locally when detailed source diagnostics are needed.

Every application request that reaches HTTP dispatch gets a bounded generated
ID, exposed as read-only `$request->requestId` and returned in `X-Request-ID`,
including host-generated rejection/error responses. Incoming request IDs are
ignored, even from trusted proxies, and application response headers cannot
replace the generated ID. IDs use a process/startup prefix and monotonic counter;
they are correlation identifiers, not secret tokens or authentication credentials.
Malformed HTTP rejected before dispatch has no request ID. The counter cannot
wrap and reuse IDs; exhaustion rejects further dispatch with 503.

`response_headers` records the selected status when the response is handed to
Hyper. `response_finished` records body bytes handed to Hyper, duration, and an
outcome: `complete`, `failed`, or `interrupted`. Streaming EOF completes the body;
a body error fails it; dropping an unfinished response interrupts it. A complete
500 response still has outcome `complete` because its HTTP body finished.
HEAD/bodyless responses report zero body bytes. Body completion is not proof of
socket delivery or browser receipt: a connection can fail after a full body was
handed off. Such failures have separate connection events. Body counts exclude
HTTP framing/TLS overhead and suppressed HEAD bytes. A HEAD/bodyless application
may continue executing after its HTTP body completes; later worker diagnostics
retain the same request ID.

Supervisor and worker diagnostic events share `requestId`; worker events also
carry numeric `workerPid` where available. Worker errors expose bounded diagnostic
codes/categories and source-relative byte spans, without arbitrary messages,
stack/source excerpts, or paths. Default host events never copy request paths,
queries, headers, cookies, session IDs, bodies, or environment contents. This also
prevents untrusted values from injecting extra log lines. There is no raw worker
stderr passthrough to the host log.

Explicit application `Log::*` stderr records are a separate policy: their existing
standard-library redaction still applies, and they are wrapped as correlated,
escaped `application_log` events. Each request can forward at most 16 records,
each retaining at most 4096 input bytes; UTF-8 truncation can use a replacement
character and sets `truncated`. Additional records are discarded. Applications
remain responsible for avoiding sensitive data in deliberately written log
messages. `Log::file` destinations remain application-managed files and do not
acquire host correlation automatically. Logging uses bounded synchronous writes
rather than an unbounded background queue; request deadlines still cover workers
blocked while forwarding logs. A blocked host stderr sink can delay the supervisor.

### Private management listener

Health checks are disabled by default. Opt in with
`--health-address 127.0.0.1:9090` or `health-address = "127.0.0.1:9090"` in TOML.
Only explicit loopback IP addresses are accepted, including `[::1]:9090`; port 0
selects a free port reported by the startup `health_listening` event (or the text
health-listening line). The value `off` disables a file-configured listener.
An exact nonzero address/port conflict with the application listener is rejected
in configuration validation. Startup still checks actual bind availability.

This is a separate listener: it never reserves application URL paths, executes
scripts, uses application worker/queue slots, or trusts forwarded identities.
GET and HEAD are supported; other methods return 405, bodies/query strings return
400, and unknown paths return 404. Responses are JSON with `Cache-Control:
no-store`. Do not expose this listener through a public reverse proxy. Loopback
binding restricts network reachability, not access by other local processes; no
management authentication system is supplied.

| Path | Meaning |
| --- | --- |
| `/livez` | 200 while the supervisor's management service is responding. Does not assert application/database health. |
| `/readyz` | 200 after startup when not draining, at least one worker has a known ready lease, and the admission queue is not full; otherwise 503. Busy workers alone do not make readiness fail if there is queue capacity. |
| `/metrics` | 200 with a fixed JSON snapshot of counters and gauges; probes do not count as application requests. |

The listener starts before worker initialization, so readiness is initially false.
Termination signals make readiness false immediately; liveness and snapshots
remain available during the bounded application drain. The health listener closes
when draining finishes. It permits at most eight concurrent connections, with
no keep-alive, a two-second header/connection lifetime, 8 KiB header buffering,
and 32 headers per request.

Snapshots contain server-lifetime saturating counters: `requests`,
`responsesComplete`, `responsesFailed`, `responsesInterrupted`, `timeouts`
(dispatch/worker request deadlines, deduplicated per request), `overload`
(admission rejections), `serverErrors` (statuses 500 and above), `bodyBytes`,
`workerFailures` (failed/cancelled/timed-out worker requests and idle worker failures),
`workerStarts` (successful initial/replacement handshakes), `workerRestarts`
(successful replacement handshakes), `workerRecycles` (count-triggered retirements),
`workerStartupFailures` (failed replacement startup attempts), `workerLoopFailures`
(unexpected worker-loop task terminations), `rejectedConnections`, `connectionTimeouts`
(header/total connection deadlines), and `connectionErrors`. Gauges are
`activeRequests`, `queuedRequests` (including reserved body reception),
`busyWorkers`, `readyWorkers` (healthy handshake leases, including busy workers),
`configuredWorkers` (target capacity), and `recoveringWorkers` (slots waiting for
retry or replacement startup); booleans are `ready`, `starting`, `draining`, and `reloading`. During a development
generation drain, readiness is false and `reloading` is true.
Snapshots read counters independently and are approximate under concurrency.
There is no per-client/URL label, retained request history, or process heap metric.

Idle child exits and unexpected IPC are monitored without waiting for an
application request. Readiness can briefly lag OS exit until the worker loop is
scheduled and the dead child is reaped; successful startup restores leases.
When all leases are lost, readiness is false throughout backoff/startup and
recovers when any replacement becomes usable. Probes themselves do not start
application work or test database dependencies. Logs distinguish `worker_idle_failure`,
`worker_recovering` (with bounded `delayMs`), `worker_startup_failed`,
`worker_loop_failed`, and `worker_recycled`; successful handshakes retain the
`worker_started` event. These events contain no script/error contents or storage
paths and use no per-client labels.

Try the [observability settings](../examples/observe-serve.toml):

```sh
scripta serve examples/web-proxy.scripta --config examples/observe-serve.toml
curl http://127.0.0.1:8080/
curl http://127.0.0.1:9090/livez
curl http://127.0.0.1:9090/readyz
curl http://127.0.0.1:9090/metrics
```

## Sessions and cookies

The Web request object can read cookies with `cookie(name)` and `cookies()`.
`Web::cookie` and `Web::deleteCookie` generate validated response cookies.
Server-side sessions use explicit `Session::start`, get/set/has/remove/all, ID
regeneration, destruction, and early close. See the [API reference](standard-library.md#sessions)
for exact semantics and limits.

For a local browser demo over plain HTTP:

```sh
scripta serve examples/web-sessions.scripta --session-cookie-secure false
```

Open `http://127.0.0.1:8080/` repeatedly: the visit count persists in the browser's
session. POST JSON to `/theme`, POST `/rotate` to regenerate the ID while keeping
data, or POST `/logout` to destroy the session and delete the preference cookie.
The example uses a deliberately non-Secure local preference cookie. It does not
implement authentication/CSRF protection; keep this demo local.

| Option | Default | Meaning |
| --- | --- | --- |
| `--session-dir` | Private temporary store | A dedicated directory; specifying it persists sessions across host restarts. Missing directories are created privately on Unix. |
| `--session-ttl-seconds` | `1800` | 1–2592000 seconds of idle lifetime, refreshed on commit. |
| `--session-max-records` | `10000` | 1–100000 stored sessions; rotation can briefly reserve one additional record. |
| `--session-max-bytes` | `65536` | 1–1048576, applied to both encoded payload and logical values/collection slots. Very small limits can reject even an empty session. |
| `--session-cookie-secure` | `true` | Set false explicitly only for development over plain HTTP. Retain true behind an HTTPS reverse proxy. |

Session cookies use the fixed name `SCRIPTA_SESSION`, host-only scope, Path=/,
HttpOnly, and SameSite=Lax. These flags are independent of forwarded headers;
HTTPS termination at Apache/Caddy needs no insecure-cookie override. Use separate
hostnames for separate applications sharing this cookie name. Ordinary cookie
helpers have their own explicit options.

The host supplies one directory to all workers. It uses OS randomness for IDs,
stable record locks, and atomic file replacement, including after worker restart.
Requests for the same session serialize until completion or `Session::close`.
Commit follows successful execution/output validation. Early close explicitly
commits even if later code fails. A network failure/deadline at the commit boundary
can leave committed state without a delivered response. Sessions set no-store
cache policy; IDs and contents are not included in host access logs.

The store must be outside public/static directories and dedicated to the app's
service account. Existing Unix stores must have private permissions (0700);
records are private files. On Windows, configure private directory ACLs. Avoid
network filesystems; cross-host/distributed session storage is not supplied.
Expired/revoked/abandoned records and staging files are reclaimed during new
session creation, under the directory lock. Requests never accept expired IDs.
The default temporary store is not reused on host restart; an abrupt host kill
can leave private temporary files for normal OS/admin cleanup.

Persistent deployment example (the service account must own this private path):

```sh
scripta serve /opt/my-app/app.scripta --session-dir /var/lib/my-app/sessions
```

## Caddy or Apache in front

Run Scripta on `127.0.0.1:8080`, then forward incoming requests. Caddy example:

```caddyfile
example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Apache virtual-host example (requires `mod_proxy` and `mod_proxy_http`; configure
TLS separately):

```apache
ProxyRequests Off
ProxyPass "/" "http://127.0.0.1:8080/"
ProxyPassReverse "/" "http://127.0.0.1:8080/"
```

The front-end handles public TLS and can serve static assets, enforce additional
limits, or balance multiple Scripta instances. Scripta needs no Apache module,
FastCGI service, or PHP installation. Choose proxy timeouts consistent with your
application deadline and configured backend connection lifetime (60 seconds by
default). These snippets must be validated in your deployment; they are not automatically installed.

### Trusted proxy metadata and allowed hosts

By default Scripta trusts no proxies. `$request->peerAddress` is the actual TCP
peer endpoint, `$request->clientAddress` is that peer's IP, `$request->scheme` is
`"http"`, and `$request->host` is the validated Host authority. Raw forwarded
headers remain accessible through `header()`/`headers()` but do not establish
identity. Scripta does not serve inbound TLS in this milestone.

Configure only proxy addresses that you operate and restrict access to the
backend. Trust is based on the socket peer IP, not on a header or source port:

```toml
trusted-proxies = ["127.0.0.1/32", "::1/128"]
allowed-hosts = ["app.example.com"]
```

The equivalent CLI options take comma-separated lists:
`--trusted-proxies 127.0.0.1/32,::1/128 --allowed-hosts app.example.com`.
An empty CLI string clears a configured list. Each option replaces its entire
file list; lists are not merged. Both default to empty and accept at most 128
entries. Trust entries are literal IPv4/IPv6 addresses or CIDRs with valid prefix
lengths. CIDRs mask any supplied host bits; a bare address matches that address
only. IPv4 and IPv4-mapped IPv6 are separate families, so configure the family
actually used by your proxy. There is no implicit trust for loopback/private
networks and no DNS lookup for trust entries.

Only an explicitly trusted immediate peer can supply these resolved fields:

| Header | Resolution policy |
| --- | --- |
| `X-Forwarded-For` | At most one header field containing 1–32 comma-separated literal IP addresses, oldest first. Spaces/tabs around entries are trimmed. Starting at the socket peer, walk right to left while the current address is trusted; the first untrusted address is the client. Discard older assertions for identity, but validate the full list. If all hops are trusted, use the leftmost address. Ports, bracketed IPv6, `unknown`, empty entries, and obfuscated identifiers are rejected. |
| `X-Forwarded-Proto` | One scalar `http` or `https` assertion from the immediate trusted proxy; otherwise the scheme remains `http`. Lists and other schemes are rejected. |
| `X-Forwarded-Host` | One scalar valid authority asserted by the immediate trusted proxy; otherwise use Host. Lists are rejected. |

Duplicate supported fields or malformed values from a trusted peer return 400
before body spooling/application execution. From an untrusted peer those three
fields are ignored for resolution, including duplicate/malformed assertions;
ordinary global header limits and ASCII validation still apply. RFC `Forwarded`,
`X-Real-IP`, `X-Forwarded-Port`, and PROXY protocol are not used. There is no
fallback between header families. The original header values are not rewritten.

The immediate trusted proxy must sanitize incoming scheme/host assertions and
supply authoritative scalar values. For chains, each trusted proxy must append
its actual immediate peer to the client-address chain, and the final trusted
proxy must supply the verified external scheme/host. Trusting a proxy does not
prove that it performed those operations. Do not copy arbitrary client-supplied
`X-Forwarded-Host`/`Proto` through a trusted peer.

Host and forwarded-host syntax is deliberately strict: ASCII DNS labels (including
`localhost` or IDNA A-labels), IPv4 literals, or bracketed IPv6 literals, with an
optional decimal port 1–65535. DNS labels are at most 63 bytes and the hostname
at most 253 bytes excluding one optional final root dot. DNS case/root dots,
IPv6 spelling, and numeric port spelling are normalized for the resolved host.
Userinfo, whitespace, URL paths, percent escapes, wildcards, IPv6 zone IDs, and
unbracketed IPv6 authorities are rejected. HTTP/1.1 still requires exactly one
valid Host field even when forwarded-host is present. HTTP/1.0 may omit Host;
its resolved host is null unless a trusted proxy supplies one.

An empty `allowed-hosts` list permits any syntactically valid resolved host.
A nonempty list checks the resolved external host after proxy processing, using
exact hostname matching with no subdomain/suffix wildcard. An entry without a
port permits any valid explicit port or no port. An entry with a port requires
that same explicit port; default ports are not inferred from the scheme.
A missing/disallowed host returns 400 before application execution. The original
backend Host must remain syntactically valid but need not be in this list when
trusted forwarded-host supplies the external authority. Use an allowlist before
using request host metadata for absolute redirects or links.

Cookie/session Secure defaults remain explicit and do not change based on the
resolved scheme. A forwarded `http` assertion never disables Secure cookies.
Use the resolved fields rather than reparsing raw headers in application code;
they describe network assertions, not authenticated users or authorization.

The [proxy example](../examples/web-proxy.scripta),
[server settings](../examples/proxy-serve.toml), and
[Caddy configuration](../examples/Caddyfile.proxy) can be paired:

```sh
scripta serve examples/web-proxy.scripta --config examples/proxy-serve.toml
caddy run --config examples/Caddyfile.proxy --adapter caddyfile
```

Replace `app.example.com` in both configurations with your domain. Caddy documents
its handling of the supported forwarded headers in its reverse-proxy reference.
If it is behind a CDN/another proxy, configure trust at Caddy as well. Validate
that deployment's actual headers; configurations that emit duplicate or list
scheme/host assertions must be adjusted to the scalar contract above. The local
regression fixture executes a sanitizing HTTP forwarder; it does not validate
Caddy installation, public certificates, or a real multi-proxy deployment.

See [Caddy reverse_proxy](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)
and the [Apache reverse-proxy guide](https://httpd.apache.org/docs/2.4/howto/reverse_proxy.html).
Persistent workers and cached parsing avoid repeated startup/parsing costs;
performance parity with PHP-FPM/OPcache has not been established by benchmarks.

## Generate static HTML or JSON on any platform

The existing CLI workflow also works: generate public files periodically and let
Apache, Caddy, IIS, or another server serve them directly. No Scripta listener is
needed for static reports. Keep only generated public content in the document root.

```scr
File::writeAtomic("public/status.json", Json::encode(["status" => "ok"]) . "\n");
```

Create `public/` before running the script and grant the front-end account read
access to generated reports. New Unix files start private; configure appropriate
permissions for public output. See the [atomic-write contract](standard-library.md#atomic-file-replacement).
CGI/FastCGI and IIS-specific dynamic handlers are not supplied. On Windows, use
reverse proxying to the Scripta HTTP host or publish generated static files.

## Password hashing in applications

`Password::hash`, `Password::verify`, and `Password::needsRehash` are available in
web workers through ordinary standard-library registration. Use them with the
existing request/session/database APIs; they do not add authentication routes or
CSRF policy. Verify credentials before setting authenticated session fields and
regenerate the session ID after successful authentication. See the
[password API](standard-library.md#passwords) for supported hashes and cost limits.

The backend's per-call memory allowance is multiplied by concurrent workers, and
the hashing computation is synchronous. Worker deadlines provide process-level
termination; they do not turn the primitive into asynchronous work. Release session
locks before unrelated slow work when the application flow permits it.

## Multipart forms and uploads

Run `scripta serve examples/web-uploads.scripta` for a browser form. It saves the
first selected file as `received-upload.bin` in the server working directory and
refuses to overwrite that path. See the [API reference](standard-library.md) for
`form()`, `uploads()`, and `UploadedFile::moveTo`. Public applications must apply
their own authentication, CSRF protection, destination selection, and content
validation. Uploaded filenames and MIME hints do not establish safe content.

The host spools multipart raw bodies and extracted files to private per-request
directories. Worker IPC carries text/metadata and private spool identifiers;
it does not carry entire upload payloads. The supervisor also owns destination
staging paths, created only after worker filesystem policy checks. Completion,
malformed input, incomplete connections, and worker deadline/failure release the
spools; workers are stopped before their active storage is removed. Disconnects
after a complete request can still allow that request to finish. A saved file
belongs to the application and is retained. Abrupt supervisor/OS termination can
leave private temporary files; normal request cleanup is not crash recovery.

| Serve option | Default | Accepted range / behavior |
| --- | --- | --- |
| `--upload-temp-dir` | Private temporary directory | Dedicated existing private directory or a new directory with an existing parent; final symlinks are rejected. Unix requires no group/other permissions. On Windows, restrict access with directory ACLs. |
| `--upload-max-file-bytes` | `8388608` | 0–67108864 per file. |
| `--upload-max-files` | `16` | 0–128, including empty browser selections. |
| `--upload-max-fields` | `128` | 0–1024 text fields; duplicates count individually. |
| `--upload-max-field-bytes` | `65536` | 0–262144 UTF-8 body bytes per text field. |
| `--upload-max-fields-bytes` | `65536` | 0–262144 total text bodies plus field-name bytes. |
| `--upload-max-header-bytes` | `8192` | 1–32768 normalized bytes per part, summing name, colon/space, value, and CRLF. Raw header intake is additionally bounded with parser read-ahead slack. At most 32 part headers. |
| `--upload-max-disk-bytes` | `268435456` | 1–1073741824 charged payload bytes across concurrent/queued requests and destination staging. |

The existing `--max-body-bytes` default remains **1 MiB** and caps the entire wire
body including multipart overhead. Raise it explicitly for larger files, for
example `--max-body-bytes 16777216`. Names are limited to 256 UTF-8 bytes, filenames
to 1024, and media hints to 256. Boundaries are 1–70 ASCII RFC 2046 characters,
with no trailing space. Text must be UTF-8 without replacement decoding; the host
rejects malformed multipart with 400, unsupported multipart encodings with 415,
request/part limits with 413, and aggregate disk exhaustion with 503. Staging
allocation failures during script execution raise a native error (500 if uncaught).

The disk budget charges raw-body bytes, extracted file bytes, and reserved save
staging bytes separately; a save can temporarily cost roughly three times the
file size. Stage reservations remain charged until request completion. The budget
counts logical payload, not filesystem blocks, metadata, or a process heap cap.
If cleanup fails, the host logs a generic warning and conservatively retains that
request’s reservation until restart rather than making the quota available again.
Metadata IPC has a separate 4 MiB ceiling. Part headers are read in bounded chunks
using Multer 3.1; text is decoded explicitly rather than using lossy parser helpers.
Keep spool directories private and separate from served static directories.

## Incremental responses and output buffers

Run `scripta serve examples/web-streaming.scripta`, then open it in a browser or
use `curl --no-buffer http://127.0.0.1:8080/`. Complete buffering remains the default.
`ob_start()` creates a binary application buffer; `ob_flush()` drains it into the
parent buffer/host writer. Only `flush()` requests HTTP transmission. Open
application buffers are not drained by `flush()`. At successful execution end,
remaining buffers drain automatically, then any final pending body is sent.

The first transport flush commits response status/headers, even when no body is
pending. It first saves/closes an active session and prevents starting a session
later in the same request. Subsequent header, status, redirect, and cookie changes
are rejected. Session data needed for streaming must be read into local variables
before flushing. A later failure cannot replace the status with 500; it is logged
and the chunked response terminates with a transport error. Already saved session
values and filesystem effects remain. Clients must handle truncated responses.

Worker IPC carries typed start, body-chunk, flush/acknowledgement, and completion
frames. Each body chunk is at most 64 KiB; each response channel holds at most one
chunk. Pipe and channel backpressure bound pending handoff data. `flush()` waits
for the supervisor to accept pending chunks, rather than confirming delivery to
a remote browser. Slow readers and blocked worker writes remain under the request
deadline; the connection lifetime is also bounded by `--connection-timeout-ms`
(60 seconds by default). Dropped streaming
clients cancel their worker request, and request-owned upload storage is cleaned
after the worker is stopped. No-body statuses/HEAD suppress streamed body data
while the worker still completes or reaches its deadline.

`--max-response-bytes` caps cumulative script-generated output, including bytes
cleaned from application buffers. Flushing never resets it. Application buffers
have a separate aggregate pending ceiling of 8 MiB (or the response ceiling when
smaller), with at most 32 levels by default. See the
[API reference](standard-library.md#output-buffers-and-flushing) for binary buffer
retrieval, cleanup, and embedding configuration.

Reverse proxies, compression, and browsers can buffer independently. For routes
that need low-latency output, disable response buffering in the proxy's route
configuration and use an appropriate Content-Type. `curl --no-buffer` disables
curl's local display buffering. A successful flush request cannot force an
intermediary or browser to display a chunk immediately. Scripta does not add an
application event loop, WebSockets, or an indefinite request lifetime.

## Raw uploads and file downloads

Use `WebRequest::bodyStream()` for raw binary POST/PUT APIs. Nonempty bodies are
received into the supervisor's private, charged spool before the script executes;
large bodies are not copied through worker IPC. `bodySize` is the measured byte
length. Each new cursor is a read-only `ByteStream`, sharing normal stream quotas,
with request-scoped authority and expiry. `bytes()`/`text()` remain convenience
whole-body reads under value limits. JSON and URL-encoded forms retain their
existing parsing semantics. All nonempty bodies now share the configured
`--upload-max-disk-bytes` ceiling, not only multipart forms; empty requests do not
create a spool. Normal completion/worker termination closes input readers before
removing storage. Abrupt supervisor/OS termination still needs private-temp cleanup.

Multipart `UploadedFile::stream()` permits content inspection before `moveTo()`.
It reveals no path and needs no ambient filesystem-read grant. Application code
must still validate formats, authentication, CSRF, and permanent destinations;
client MIME/filenames are hints. Host-level upload errors reject the request
before executing the script, rather than exposing partially accepted files.

`Web::download(fileStream, filename)` supplies a safe attachment response with
UTF-8 filename encoding and `nosniff`. It validates file authority, basename,
remaining transfer/output budgets, and untouched body/buffer state before
committing metadata. Source data is sent in bounded chunks with backpressure;
HEAD avoids reading the payload. Close the stream and `exit 0` afterward.
Whole-file responses do not advertise resumable ranges. See
[the transfer example](../examples/web-transfers.scripta) and the
[PHP comparison/API contract](standard-library.md#upload-and-download-contract).

Request workers retain outgoing socket capabilities but remove `network.listen`
and `network.bind`; inbound socket servers run in explicitly hosted CLI/embedding
contexts rather than inside a per-request worker.
