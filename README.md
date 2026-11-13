<h1 align="center">
  <img src="docs/assets/header.svg" alt="Scripta — lightweight scripting for everyday automation" width="960">
</h1>

<p align="center">
  A standalone CLI scripting language for files, commands, system utilities, and APIs.<br>
  PHP-inspired syntax · Explicit value conversions · Written in Rust
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#build-from-source">Build from source</a> ·
  <a href="#linux-server-automation">Server automation</a> ·
  <a href="#web-server-use">Reports and dashboards</a> ·
  <a href="docs/standard-library.md">Standard library</a> ·
  <a href="examples/README.md">Examples</a>
</p>

## Project status

**0.1.0 release preparation is implemented. A public release has not been published.**
On macOS ARM64, all 229 tests passed in both debug and distribution profiles,
along with formatting, Clippy, the Rust 1.85 source check, archive checksum and
reproducibility checks, and an offline test of the packaged executable.
The other targets still need their native validation.

| Target | Current validation |
| --- | --- |
| macOS ARM64 | Verified locally on macOS 27.0.1; macOS 15 validation pending |
| macOS x86-64 | Native validation pending |
| Linux x86-64 | Native validation pending |
| Linux ARM64 | Native validation pending |
| Windows x86-64 | Native validation pending |

Build instructions below describe the configured targets; they do not certify an
untested platform. See the [compatibility matrix](docs/compatibility.md) for OS
baselines and evidence, and the [release checklist](docs/release-checklist.md)
for the remaining release work. Candidates are unsigned and not notarized.

## What Scripta provides

Scripta focuses on command-line tools, scheduled jobs, server automation, and
report generation. Run scripts directly or through cron and systemd; publish
generated HTML or JSON through an existing web server when you need a dashboard.

- **Everyday automation:** filesystem operations, literal command arguments,
  bounded process output, timeouts, and local system metrics.
- **Structured data and APIs:** lists, maps, JSON, HTTPS requests, UTC dates,
  and structured local logging.
- **Reusable scripts:** default/variadic parameters, anonymous functions, lexical
  captures, local `require` / `require_once`, and versioned Git/local packages.
- **Developer tools:** syntax checks, a comment-preserving formatter, error codes,
  source locations, and runtime stack traces.
- **Optional SQLite:** build with `--features sqlite` for parameterized SQL,
  transactions, and bounded query results. See the [SQLite extension](docs/sqlite.md).
- **A standalone interpreter:** the installed executable uses platform system
  libraries. Local scripts need no Python, Node.js, PHP, Docker, Rust, or network.

Scripta is dynamically typed with strict operations and explicit conversions.
Its syntax is inspired by PHP; the [language specification](docs/language-spec.md)
defines what it supports. String interpolation and script-defined classes remain
unimplemented.

## Quick start

Save this as `hello.scr`. Every script, including an included file, starts with
`<?scr`; a closing tag is optional.

```scr
<?scr
$name = "Scripta";
echo "Hello, " . $name . "!\n";
```

With the interpreter installed on PATH:

```sh
scr run hello.scr
# Hello, Scripta!
```

`scr hello.scr` is equivalent. Arguments after the script path are available
through `Cli::args()` and `Cli::arg()`. A `.scr` file remains source code: run it
through `scr`; building the interpreter does not compile each script into an app.

### Try the playground apps

From the repository root (the directory containing `Cargo.toml`), build once and
run any of these small apps:

```sh
cargo build -p scr-cli --locked
./target/debug/scr run playground/expenses.scr
./target/debug/scr run playground/table.scr
./target/debug/scr run playground/scores.scr
./target/debug/scr run playground/arrays.scr
./target/debug/scr run playground/strings.scr
./target/debug/scr run playground/collections.scr
./target/debug/scr run playground/log-analyzer.scr playground/sample-events.txt
./target/debug/scr run playground/errors.scr
./target/debug/scr run playground/functions.scr
./target/debug/scr run playground/reporting.scr
./target/debug/scr run playground/streaming.scr playground/sample-events.txt
```

| Script | What to try |
| --- | --- |
| [expenses.scr](playground/expenses.scr) | Edit the expense map; `foreach` prints each item and calculates the total. |
| [table.scr](playground/table.scr) | Change `$number`; a `while` loop prints its multiplication table. |
| [scores.scr](playground/scores.scr) | Change the scores; `if` / `elseif` / `else` assigns a result to each name. |
| [arrays.scr](playground/arrays.scr) | Build a list with `[] =` and assignment at its length, replace an item, and append to a list inside a map. |
| [strings.scr](playground/strings.scr) | Parse, normalize, search, and format a Unicode status record with the `String` helpers. |
| [collections.scr](playground/collections.scr) | Filter, sort, and summarize records using list helpers and safe map lookups. |
| [log-analyzer.scr](playground/log-analyzer.scr) | Analyze [sample events](playground/sample-events.txt), count log levels, list sorted services, and report malformed records. |
| [errors.scr](playground/errors.scr) | Recover from indexing, arithmetic, and operand-type failures; add `--uncaught` to demonstrate a diagnostic and exit status 1. |
| [streaming.scr](playground/streaming.scr) | Stream log records from a file or stdin (`-`) with a line-size limit; file cleanup uses `finally`. |
| [reporting.scr](playground/reporting.scr) | Summarize durations with Math helpers and bounded string formatting. |
| [functions.scr](playground/functions.scr) | Try defaults, typed variadics, anonymous callbacks, captured counters, and independent collection defaults. |

After editing, check a script with
`./target/debug/scr check playground/expenses.scr`, then run it again. There is no
separate compilation step for `.scr` files. With an installed interpreter, use
`scr run playground/expenses.scr` instead.

## Run a standalone executable

Obtain an archive **only after its target has been verified**, check its SHA-256
sidecar, and extract it. No public download URL is available yet. A local macOS
ARM64 candidate is produced under `dist/` during release preparation.
See [installation and uninstallation](docs/install.md) for checksums, PATH setup,
and platform security prompts.

Run these from the extracted package directory. Each package includes `examples/`:

| System | Inspect the executable | Run a script |
| --- | --- | --- |
| macOS | `./scr version --verbose` | `./scr run examples/hello.scr` |
| Linux | `./scr version --verbose` | `./scr run examples/hello.scr` |
| Windows (PowerShell) | `.\scr.exe version --verbose` | `.\scr.exe run .\examples\hello.scr` |

You can keep the interpreter anywhere and give it an absolute script path. In
PowerShell, invoke a quoted executable path with `&`, for example:

```powershell
& 'C:\Tools\Scripta\scr.exe' run 'C:\Scripts\hello.scr'
```

After placing the executable on PATH, `scr run hello.scr` works on all three
systems. An explicit `Http` call needs the requested endpoint; `Process::run`
needs the program the script chooses to launch.

## Build from source

Start in the repository root, whether obtained through Git or a source archive.
A source build needs **Rust/Cargo installed through rustup** and a native C
compiler/linker. Follow the [official Rust installation guide](https://doc.rust-lang.org/book/ch01-01-installation.html).
The checkout pins Rust **1.99.1** in `rust-toolchain.toml`; rustup installs that
compiler when needed. Rust **1.85** is the separately checked source minimum.
The initial toolchain/dependency downloads require a network connection; running
an installed interpreter on local scripts does not.

### macOS

Install Apple's Command Line Tools (`xcode-select --install`) if absent, then
install Rust through rustup. These commands build natively on Apple Silicon:

```sh
export MACOSX_DEPLOYMENT_TARGET=15.0
cargo test --workspace --target aarch64-apple-darwin --locked
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked
./target/aarch64-apple-darwin/dist/scr version --verbose
./target/aarch64-apple-darwin/dist/scr run examples/hello.scr
```

On an Intel Mac, replace **every** `aarch64-apple-darwin` above with
`x86_64-apple-darwin`. This is a native build, not a universal binary. Older macOS
versions and running Intel binaries through Rosetta have not been verified.

### Linux

Install GCC or Clang and the development libraries/linker for your distribution,
then install Rust through rustup. On Ubuntu/Debian, the usual linker prerequisite
is `build-essential` (`sudo apt-get install build-essential`).

```sh
cargo test --workspace --locked
cargo build -p scr-cli --profile dist --locked
./target/dist/scr version --verbose
./target/dist/scr run examples/hello.scr
```

Cargo selects the native host architecture: x86-64 on an x86-64 host, ARM64 on an
ARM64 host. Configured Linux distribution candidates use GNU/glibc targets built
on Ubuntu 22.04. musl/Alpine and ARM32 are outside the release matrix.

### Windows

Install Rust through rustup using the **MSVC** toolchain and Visual Studio Build
Tools with the **Desktop development with C++** workload and Windows SDK.
Open PowerShell in the repository root:

```powershell
cargo test --workspace --target x86_64-pc-windows-msvc --locked
cargo build -p scr-cli --profile dist --target x86_64-pc-windows-msvc --locked
.\target\x86_64-pc-windows-msvc\dist\scr.exe version --verbose
.\target\x86_64-pc-windows-msvc\dist\scr.exe run .\examples\hello.scr
```

Use the explicit `--target` on Windows Cargo build/test/install commands. The
project enables the static MSVC CRT for that target; separating host builds keeps
that setting away from compiler plugins. Windows ARM64 is outside the matrix.

### Install a source build on PATH

On macOS/Linux, `cargo install --path crates/scr-cli --locked` installs into
Cargo's bin directory. On Windows use:

```powershell
cargo install --path crates/scr-cli --target x86_64-pc-windows-msvc --locked
```

Restart the terminal if PATH was just updated, then run `scr version --verbose`.
For a downloaded binary, follow the [per-user installation steps](docs/install.md)
instead. The `dist` profile optimizes and strips the executable while preserving
panic unwinding and overflow checks; see [release procedures](docs/releases.md).

## Linux server automation

Copy or clone the repository onto the Linux server and follow the Linux build
instructions above **on that server**. A macOS executable cannot run on Linux.
After the build passes, install the interpreter system-wide:

```sh
sudo install -m 755 target/dist/scr /usr/local/bin/scr
/usr/local/bin/scr version --verbose
/usr/local/bin/scr run examples/server-health.scr /
```

The last command prints server metrics as JSON. Once built, the interpreter does
not need Rust/Cargo to run scripts. Keep the scripts on the server too, including
any files they `require`. Use absolute paths and run under the account whose files
and commands the automation needs to access.

For recurring jobs, use cron. For example, with the checkout at `/opt/scripta`,
create a log directory under the automation user's home, then open that user's
crontab:

```sh
mkdir -p "$HOME/scripta-logs"
crontab -e
```

Add this single line to collect server health every five minutes:

```cron
*/5 * * * * cd /opt/scripta && /usr/local/bin/scr run /opt/scripta/examples/server-health.scr / >> "$HOME/scripta-logs/health.log" 2>&1
```

Replace `/opt/scripta` with the actual checkout path. This appends a JSON report
on each run; rotate the log for ongoing use. Cron has a smaller environment than
an interactive shell, so explicitly supply any environment variables and command
paths your script needs. For a continuously running script, use a systemd service
with an explicit `User`, `WorkingDirectory`, and `ExecStart` rather than keeping
an SSH terminal open. See the [crontab reference](https://man7.org/linux/man-pages/man5/crontab.5.html).

## CLI reference

| Command | Purpose |
| --- | --- |
| `scr run script.scr` | Execute a script; `scr script.scr` is shorthand. |
| `scr run - [arguments...]` | Read script source from stdin through EOF and execute it. |
| `scr check script.scr` | Validate tags, complete syntax, declarations, and control flow without execution. |
| `scr fmt script.scr` | Format indentation and spacing while preserving comments and string literals. |
| `scr fmt --check script.scr` | Check formatting without writing; useful in CI. |
| `scr version` | Print the interpreter version. |
| `scr version --verbose` | Show version, build target, compiler, profile, and supplied commit metadata. |
| `scr help` | Show usage; `scr help run`, `check`, or `fmt` shows command help. |

`check` is a syntax/declaration check, not complete static type checking. It does
not execute or load required files. `fmt` is idempotent and keeps an optional
closing tag. Use `scr run -- help` for a script whose path matches a command name.

Exit codes: **0** for success, **1** for script/file failures or formatting that
needs changes, **2** for CLI usage errors, and **130** for interruption.
A script can explicitly return a code from 0 through 255 with `exit(code)`.

### Shell pipelines

A file-backed script can consume piped data without printing a prompt:

```scr
<?scr
while (($line = Cli::readLine()) !== null) {
    echo String::upper($line) . "\n";
}
```

`Cli::readLine(maxLineBytes)` defaults to a 1 MiB raw line limit, including LF/CRLF.
It returns `null` at EOF and an empty string for a blank line. Input failures
raise exceptions and disable further stdin reads through `readLine` or `prompt`
in that runtime registration. See the [input contract](docs/standard-library.md#script-arguments-and-prompts).

On macOS/Linux, try the streaming example with piped data:

```sh
cat playground/sample-events.txt | scr run playground/streaming.scr -
printf '%s\n' '<?scr echo "Hello from stdin!\n";' | scr run -
```

`scr run -` consumes stdin as UTF-8 source through EOF, subject to
`SCR_MAX_SOURCE_BYTES`. Relative includes and package lookup start from the working
directory; diagnostics identify the source as `<stdin>`. Stdin is then at EOF for
script input. Use a file-backed script when stdin should carry data. A literal
file named `-` remains accessible with `scr run -- -` or `scr run ./-`.
`run`, `check`, and `fmt` also accept `--help` or `-h` before the source path.

## Web-server use

Scripta's dashboard workflow follows its CLI focus: run a script to collect data
and generate HTML or JSON, then serve the generated files with Caddy or Apache.
Use cron to refresh the output independently of browser requests. Visitors see
the latest generated report, and browser-side JavaScript can poll the JSON for
updates.

`Http` makes outgoing requests. Scripta has no built-in HTTP server, PHP-style
request handler, or FastCGI service. A web runtime or FPM equivalent is outside
the current project direction. The
[static-output guide](docs/web-servers.md#generate-static-html-or-json-on-any-platform)
shows report generation on macOS, Linux, and Windows.

### A dashboard with Caddy or Apache on Linux

For a dashboard that shows periodically refreshed data, have Scripta generate
HTML and JSON, schedule that generation with cron, and let the web server serve
the output. Start with `report.scr` in the
[static-output guide](docs/web-servers.md#generate-static-html-or-json-on-any-platform).
For example, keep the script at `/opt/scripta/app/report.scr` and generate its
`public/` output under `/srv/scripta-dashboard`:

```sh
# Create this directory as administrator, then grant the automation user ownership.
sudo mkdir -p /srv/scripta-dashboard/public
sudo chown -R "$(id -un):$(id -gn)" /srv/scripta-dashboard
cd /srv/scripta-dashboard
/usr/local/bin/scr run /opt/scripta/app/report.scr
```

Ensure the web-server account can traverse the directory and read the generated
files. Schedule the same `cd` and `scr run` command in cron to refresh the data.
For concurrent readers, generate replacement files beside the published files
and rename them into place after successful generation. The sample report is a
starting point: add `System` metrics, HTML, and browser-side JavaScript that polls
`status.json` to turn it into a dashboard.

For **Caddy**, add a site block to your Caddyfile:

```caddyfile
dashboard.example.com {
    root * /srv/scripta-dashboard/public
    header Cache-Control "no-store"
    file_server
}
```

Replace the hostname with your domain and point its DNS to the server. Validate
and reload your Caddy configuration. See Caddy's
[file server documentation](https://caddyserver.com/docs/caddyfile/directives/file_server).

For **Apache**, use this in an enabled virtual host:

```apache
<VirtualHost *:80>
    ServerName dashboard.example.com
    DocumentRoot /srv/scripta-dashboard/public
    <Directory /srv/scripta-dashboard/public>
        Options -Indexes
        AllowOverride None
        Require all granted
    </Directory>
</VirtualHost>
```

Validate and reload Apache using your distribution's tools (for example,
`sudo apache2ctl configtest` and `sudo systemctl reload apache2` on Ubuntu/Debian).
Configure HTTPS for public access. See Apache's
[document root reference](https://httpd.apache.org/docs/2.4/mod/core.html#documentroot).
These configurations publish the generated files to visitors; keep private
metrics behind your existing authentication or private network.

Keep source scripts, logs, and credentials outside the public directory; serve
only the generated output. These server configurations are documentation
examples and have not been deployment-tested in this checkout.

## Standard library

The [complete reference](docs/standard-library.md) documents all **105 functions**,
return values, native objects, limits, and errors.

| Modules | Use |
| --- | --- |
| `File`, `Dir`, `Path` | Read, write, stream bounded lines, list, copy, move, and organize files. |
| `Process` | Run literal arguments, capture bounded output, manage child processes. |
| `Env`, `Cli` | Environment values, script arguments, options, and prompts. |
| `System` | Platform, CPU sampling, system memory, disk usage, and host details. |
| `Date`, `Log` | UTC timestamps, parsing, sleep, and redacted local JSON logs. |
| `Json`, `Http` | JSON conversion and bounded HTTP/HTTPS requests. |
| `String` | Trim, split/join, search, replace, take substrings, convert case, and format scalar values with `{}`. |
| `Math` | Type-preserving abs, min/max, floor, ceil, and round (ties away from zero). |
| `List`, `Map` | Slice, remove, concatenate, sort, test membership, construct maps, and safely look up keys. |
| Global functions | Explicit conversions, `type`, `len`, and cancellable `sleep`. |

Local script reuse uses `require "lib/helpers.scr"` or
`require_once "lib/config.scr"`. Relative paths resolve against the including file;
includes share their caller's current scope. See the [module contract](docs/language-spec.md#local-script-loading).

Shared libraries can be installed with the built-in package manager:

```sh
scr init
scr add helpers --path ./lib/helpers
# Git sources can select version tags or a specific revision:
scr add helpers --git https://example.com/team/helpers.git --version '^1.2'
scr install --locked
```

Load an installed package with an alias to keep its names isolated:

```scr
<?scr
require_once "helpers" as Helpers;
echo Helpers::greeting();
```

Use `Helpers::functionName()` to call module functions. Aliased packages have their
own functions and variables; different libraries can use identical helper names.
`require_once` shares an initialized instance across aliases of the same canonical
entry file. Plain `require` executes a fresh instance. Unaliased includes retain
the existing shared-scope behavior. See [alias semantics](docs/packages.md#package-aliases-and-isolation).

Commit `scripta.toml` and `scripta.lock`; ignore `.scripta/`.
See [packages](docs/packages.md) for package metadata, transitive dependencies,
offline installs, and update behavior.

## Examples

Start with [hello.scr](examples/hello.scr), [variables.scr](examples/variables.scr),
[loops.scr](examples/loops.scr), and [functions.scr](examples/functions.scr).
The [example guide](examples/README.md) also covers file organizing, log cleanup,
directory backups, server health, HTTP requests, and command execution.
File-changing automation examples preview their work and require `--apply` to
perform changes. The older `organize.scr` example moves files immediately.

## Limits and diagnostics

Errors include codes, source locations, excerpts, and runtime traces. Diagnostics
mask literal/comment contents and redact known credentials. Execution limits can
be configured for trusted automation; for example, on macOS/Linux:

```sh
SCR_MAX_STEPS=1000000 SCR_MAX_VALUE_BYTES=8388608 SCR_MAX_SCOPES=1024 scr run automation.scr
```

In PowerShell set `$env:SCR_MAX_STEPS = '1000000'` before invoking `scr`.
Source files default to an 8 MiB ceiling and value nesting is bounded at 128.
Optional value/scope budgets are logical limits, not a hard process RSS ceiling.
See the [performance and safety report](docs/performance.md) and
[release limitations](docs/releases.md#limitations-for-010).

## Documentation

[Language specification](docs/language-spec.md) ·
[Standard library](docs/standard-library.md) ·
[Installation and uninstallation](docs/install.md) ·
[Compatibility](docs/compatibility.md) ·
[Architecture](docs/architecture.md) ·
[Development](docs/development.md) ·
[Release procedures](docs/releases.md) ·
[Release checklist](docs/release-checklist.md)

## License

Scripta is distributed under the [MIT license](LICENSE). Packaged candidates
include dependency licenses and notices.
