# Using Scripta with a web server

Scripta 0.1.0 is a CLI interpreter. It can generate content or be launched by an
existing server, but has no built-in listener, HTTP request/response framework,
FastCGI service, sessions, or automatic PHP-style `.scr` execution. `Http::get`
and related APIs are HTTP clients. Putting a script in a web root does not make
it execute and can expose its source if the server serves the file directly.

## Generate static HTML or JSON on any platform

Run a trusted script and serve only its output through your existing Apache,
Nginx, IIS, or other static-file server. This works without adding a Scripta
request handler. The commands below generate local files; they do not install or
start a web server. A public server still needs its normal configuration.

Save `report.scr` outside the directory you will publish:

```scr
<?scr
$report = ["status" => "ok", "generatedAt" => Date::now()];
File::writeAtomic("public/status.json", Json::pretty($report) . "\n");
File::writeAtomic("public/index.html", "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Scripta report</title><h1>Scripta report</h1><p>Status: OK</p></html>\n");
```

From the directory containing `report.scr`, create the output directory and run
it with an installed executable. On macOS/Linux:

```sh
mkdir -p public
scr run report.scr
# Public reports only: allow the web-server account to read these new files.
chmod 644 public/status.json public/index.html
```

On Windows in PowerShell:

```powershell
New-Item -ItemType Directory -Force public | Out-Null
scr run .\report.scr
```

If scr is not on PATH, substitute its absolute path; PowerShell needs `&` before
a quoted executable path. Configure your existing server's document root or
static-site directory to the absolute path of `public/`:

| Server | Publish |
| --- | --- |
| Apache or Nginx on macOS/Linux | The generated `public/` directory as the site's static document root. |
| IIS on Windows | The generated `public/` directory as a site's physical path, with static content enabled. |

Visit `/` and `/status.json` on that configured site. The files remain unchanged
until the script runs again. For recurring generation, invoke the same fixed
command from your existing scheduler; scr does not create a background daemon.
Keep scripts, logs, credentials, and the source checkout outside the document
root. Only publish fields intended for visitors. `File::writeAtomic` publishes a
complete replacement of each file, so a concurrent reader sees old or new contents.
New Unix files start private; the first-run chmod above makes these public reports
readable, and later replacements preserve those ordinary permission bits. Windows
files inherit directory ACLs; configure the web-server account's access on that
directory. Replacement does not preserve custom ACLs or ownership. If HTML and JSON
must update as one consistent set, use a versioned output directory and an external
publication mechanism; the two writes are independent. See the
[atomic-write contract](standard-library.md#atomic-file-replacement).

## Optional Apache CGI demonstration on macOS/Linux

This is a **manual, local-only integration example**, not a tested Scripta web
hosting feature. It requires an independently installed/configured Apache with
CGI enabled and a Unix shell for the launcher. The scr executable itself still
has no shell or web-server dependency. A new interpreter process starts for each
request. See Apache's [CGI guide](https://httpd.apache.org/docs/2.4/howto/cgi.html)
for enabling `mod_cgid` with threaded Unix MPMs or `mod_cgi` with non-threaded MPMs.

Use a dedicated layout outside the document root; substitute your actual absolute
paths. The Apache worker must be able to read the script and execute scr:

```text
/opt/scripta/bin/scr
/opt/scripta/app/status.scr
/opt/scripta/cgi-bin/status.cgi
```

Save this as `/opt/scripta/app/status.scr`. Here the script explicitly emits the
CGI headers, a blank line, and a JSON body:

```scr
<?scr
echo "Content-Type: application/json; charset=utf-8\r\n";
echo "Cache-Control: no-store\r\n\r\n";
echo Json::encode(["status" => "ok"]) . "\n";
```

Save the following fixed launcher as `/opt/scripta/cgi-bin/status.cgi`. The shebang
belongs to this shell wrapper; the Scripta file still begins with `<?scr`.

```sh
#!/bin/sh
cd /opt/scripta/app || exit 1
export SCR_MAX_STEPS=100000
export SCR_MAX_VALUE_BYTES=1048576
export SCR_MAX_SCOPES=128
exec /opt/scripta/bin/scr run /opt/scripta/app/status.scr
```

Make the launcher and interpreter executable (`chmod 755` on those two files).
Keep the script and launcher owned by the administrator/deployer and unwritable
by the web-server account. In an existing Apache virtual host with CGI enabled,
add a local-only route:

```apache
ScriptAlias "/scripta-demo/" "/opt/scripta/cgi-bin/"
<Directory "/opt/scripta/cgi-bin">
    Options +ExecCGI -Indexes
    Require local
</Directory>
```

Have the server administrator validate and reload that configuration. From the
same machine, request `/scripta-demo/status.cgi` on the configured virtual host;
the body should be `{"status":"ok"}`. `Require local` intentionally restricts
this demonstration to local clients. Configure a server-side timeout as well;
Apache's [CGIDScriptTimeout](https://httpd.apache.org/docs/2.4/mod/mod_cgid.html#cgidscripttimeout)
limits waits for CGI output but is not a total wall-clock execution budget.
Scripta's step limit also does not bound time spent blocked in native operations.

This wrapper always runs the fixed script. It does not accept uploaded scripts,
map request URLs to source paths, or interpret query parameters as commands.
It does not implement request-body parsing, authentication, or application error
responses. Errors go to stderr/server logs; failures after response headers have
been sent cannot retroactively change the HTTP status. Review an application
adapter before exposing dynamic functionality beyond this demonstration.

The wrapper's emitted bytes can be tested locally, but an actual Apache request,
permissions, modules, timeouts, and OS integration still require server testing.
No Apache deployment was validated during this documentation change.

## Existing application servers and Windows

An existing backend can launch scr or scr.exe with an argument array, a fixed
trusted script path, a fixed working directory, and a restricted environment.
The backend should capture stdout/stderr separately, enforce elapsed-time/output
limits, and choose the HTTP status after the child finishes. Keep diagnostics
private and return only the intended body. Do not pass raw HTTP input to a shell
or select an executable/script path from request parameters.

That adapter is application code and is not supplied with Scripta. There is no
validated IIS handler or Windows CGI configuration in this release. On Windows,
use the static-output procedure above, or integrate with your existing backend
and test that adapter independently. No new HTTP daemon is needed for local CLI
scripts, and no `scr serve`, `scr -S`, or PHP-FPM configuration is available.
