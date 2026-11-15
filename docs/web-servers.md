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

Response output is buffered before transmission. Uncaught errors, nonzero script
exit, or output overflow discard application output and return a generic `500`.
Error responses may exceed a deliberately tiny application output ceiling by a
small fixed amount. Detailed diagnostics and status/duration access logs go to
stderr, without request bodies or query strings. `exit 0` finishes a valid response.
HEAD suppresses the transmitted body; statuses 204, 205, and 304 also send no body.
Application code should recognize HEAD when it wants it to follow a GET route.

Ctrl+C / termination signals stop new connections and allow a bounded drain of
existing work before terminating remaining workers. Keep-alive connections can
consume that drain interval. The parent is intended to run under an OS service
manager for restart after host failure. Worker stdin monitoring makes workers exit
when their parent pipe closes, including while a request is blocked.

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
