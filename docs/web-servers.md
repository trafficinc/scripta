# Serving web requests

Scripta can receive HTTP requests with `scripta serve` and execute one fixed entry
script for each request. `Http::get` and related functions remain outgoing HTTP
clients. The `Web` module is available only inside request execution; CLI scripts
continue to work as before. No new language syntax is needed.

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
<?scripta
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

## Runtime and limits

The HTTP host uses Hyper and Tokio. Persistent worker **processes** each parse the
entry script once and execute it in a fresh runtime for every request. Variables,
module state, environment snapshots, response headers, input/output, and native
resources belong to that request. Workers do not launch a new interpreter for
every request. Required/imported scripts are still loaded per request; the entry
script cache is an AST cache, not bytecode or a JIT.

The parent keeps one startup snapshot of the entry script, also used by replacement
workers. Restart the service to deploy entry-script changes. Includes and data
files are not snapshotted: use an immutable application directory and a graceful
service replacement for consistent multi-file deployments.

| Option | Default | Bounds / meaning |
| --- | --- | --- |
| `--host` | `127.0.0.1` | IP address; use a local reverse proxy for public HTTPS. |
| `--port` | `8080` | 0–65535; 0 selects an available port and prints it to stderr. |
| `--workers` | CPU count, capped at 4 | 1–64 persistent processes; one active request per worker. |
| `--queue` | `32` | 1–4096 waiting requests, including reservations while receiving bodies. |
| `--max-connections` | `128` | 1–4096 open HTTP connections. Excess connections close immediately. |
| `--max-body-bytes` | `1048576` | 0–67108864, checked for both fixed and chunked bodies. |
| `--max-response-bytes` | `8388608` | 1–67108864 application body bytes, including `echo` and `Web::write`. |
| `--timeout-ms` | `10000` | 1–3600000, covering body reception, queue wait, worker recovery and execution. |

Additional fixed limits: request URI 8192 encoded bytes, header buffering 32 KiB,
100 headers, response headers 32 KiB/100 entries, header-read timeout 10 seconds,
and maximum connection lifetime 60 seconds (including response writes). A longer
request timeout does not extend that connection lifetime. HTTP/1.1 keep-alive is
supported within that lifetime. Request trailers, CONNECT, upgrades, WebSockets,
and streaming responses are not supported. Non-ASCII header values are rejected
on input; request bodies can contain arbitrary bytes.

The server reserves queue capacity before buffering bodies. Overload returns
`503`, oversized bodies `413`, malformed requests `400`, worker transport failures
`502`, and request deadlines `504`. A worker that fails or exceeds its deadline is
terminated and replaced lazily for subsequent work. Queued requests that expire
are discarded. Already completed external side effects are not rolled back.

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
small fixed amount. Detailed diagnostics and status/duration access logs go to
stderr, without request bodies or query strings. For streamed responses, the
status/duration log records header readiness, rather than full body completion. `exit 0` finishes a valid response.
HEAD suppresses the transmitted body; statuses 204, 205, and 304 also send no body.
Application code should recognize HEAD when it wants it to follow a GET route.

Ctrl+C / termination signals stop new connections and allow a bounded drain of
existing work before terminating remaining workers. Keep-alive connections can
consume that drain interval. The parent is intended to run under an OS service
manager for restart after host failure. Worker stdin monitoring makes workers exit
when their parent pipe closes, including while a request is blocked.

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
application deadline and 60-second backend connection lifetime. These snippets
must be validated in your deployment; they are not automatically installed.

Forwarded headers are ordinary untrusted input to `WebRequest`; Scripta does not
infer a trusted client identity or external scheme from them. No trusted-proxy
configuration is implemented yet. Prefer relative redirects. If your application
uses forwarded host/scheme/client fields, restrict backend access and implement
an explicit trusted-proxy policy first.

See [Caddy reverse_proxy](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)
and the [Apache reverse-proxy guide](https://httpd.apache.org/docs/2.4/howto/reverse_proxy.html).
Persistent workers and cached parsing avoid repeated startup/parsing costs;
performance parity with PHP-FPM/OPcache has not been established by benchmarks.

## Generate static HTML or JSON on any platform

The existing CLI workflow also works: generate public files periodically and let
Apache, Caddy, IIS, or another server serve them directly. No Scripta listener is
needed for static reports. Keep only generated public content in the document root.

```scr
<?scripta
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
deadline; the connection lifetime is also bounded to 60 seconds. Dropped streaming
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
