<h1 align="center">
  <img src="docs/assets/scripta-banner.png" alt="Scripta — a standing black American bison wrapped in a glowing beam of light. Lightweight scripting for everyday automation." width="960">
</h1>

<p align="center">
  A standalone scripting language for files, commands, system utilities, and APIs.<br>
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

# Scripta

**The simplicity of PHP. The practicality of shell scripting. Powered by Rust.**

**One executable. Write a script and run it.**

Scripta is a lightweight, standalone scripting language for everyday automation. Download the executable for your platform, save your code in a `.scripta` file, and run it with `scripta run your-script.scripta`.

Built in Rust with familiar PHP-inspired syntax, Scripta combines filesystem operations, system commands, HTTP APIs, JSON processing, and built-in SQLite into one practical scripting environment.

Local scripts need no Python, Node.js, PHP, Docker, Rust, or separate compilation step. The interpreter uses platform system libraries; scripts that launch external commands still need those programs installed.

**What can you build with Scripta?**

- **System automation** — Automate repetitive tasks, manage files, execute commands, and monitor system resources.
- **Server administration** — Create maintenance scripts, scheduled jobs, health checks, and deployment utilities.
- **CLI tools** — Build custom command-line applications with structured arguments and reusable functions.
- **API integrations** — Work with HTTPS requests, JSON data, and external services.
- **Data processing** — Process files, generate reports, and store information using built-in SQLite.
- **Web utilities** — Generate dashboards, build HTTP-powered tools, and automate web workflows.

### Why Scripta?

Scripta bridges the gap between shell scripts that become difficult to maintain and general-purpose languages that can feel excessive for straightforward automation.

- **Simple setup:** one interpreter executable runs your scripts, with no separate language runtime to install or per-script compilation step.
- **Useful tools built in:** work with files, commands, HTTP APIs, JSON, and SQLite. Standard release packages include SQLite without a separate database server or library installation.
- **Predictable type rules:** strict runtime operations and explicit conversions help catch incompatible values; optional function annotations check arguments and return values at runtime.
- **An integrated workflow:** format scripts, run native Scripta tests, and check syntax with the same executable. Errors include source locations and runtime stack traces.
- **Multiple platforms:** prebuilt packages target Linux x86-64, Windows x86-64,
  and Apple Silicon macOS. Linux ARM64 is available from source. See
  [compatibility](docs/compatibility.md) for architectures and OS baselines.

### ~ *SCRIPTA IS STILL IN BETA* ~

Please do not use this in professional use, but only for hobby projects. It is unknown if Scripta will become usable for production use-cases, but I hope so. If you find any bugs please report them in the Issues tab! We are actively looking at them!

## Benchmarks

<!-- BEGIN LANGUAGE BENCHMARKS -->
Median whole-process wall time in milliseconds; **lower is better**.

| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |
| --- | --- | ---: | ---: | ---: |
| Numeric loop | 1,000,000 iterations | 36.153 | 69.978 | 21.038 |
| Recursive Fibonacci | fib(28), no memoization | 42.970 | 43.367 | 35.090 |
| List pipeline | 2,000 items × 200 passes | 41.673 | 36.650 | 51.480 |
| String concatenation | 100,000 × 8-byte appends | 567.528 | 521.676 | 17.067 |
| Dictionary operations | 256 keys; 40,960 updates; 128 copy-removals | 28.710 | 22.072 | 29.014 |
| Class dispatch | 200,000 method calls | 31.349 | 29.293 | 57.304 |
| Regex match | 20,000 matches attempted | 28.239 | 18.533 | 10.855 |
| JSON roundtrip | 20,000 encode/decode pairs | 39.201 | 61.953 | 52.521 |
| Function calls | 200,000 calls | 30.110 | 28.276 | 15.072 |
| File processing | 20,000 lines; 260,000 bytes | 29.281 | 58.268 | 15.350 |
| CSV → JSON | 10,000 quoted UTF-8 records | 36.130 | 30.833 | 35.744 |
| SQLite transaction | 10,000 inserts + aggregate query (in-memory) | 36.434 | 27.516 | 15.363 |
| Startup | Empty script | 26.989 | 14.323 | 4.496 |

Measured locally,
with 15 samples after 3 warmups per workload/runtime.
Scripta uses an optimized native release build. PHP runs without php.ini,
with CLI OPcache, PHP JIT, and PCRE JIT disabled; Python runs in isolated mode.
Timings include startup, parsing, execution, and output validation work inside each script;
filesystem caches are warm. These workloads compare their implementations and runtime APIs,
including collection copying and regex compilation/caching; they are not an overall language ranking.
See the [methodology and reproduction commands](benchmarks/comparison/README.md).
Raw samples and environment details are retained locally in ignored benchmark JSON files.
<!-- END LANGUAGE BENCHMARKS -->

## Try Scripta Playground

Try Scripta in a local browser editor with syntax highlighting, example scripts,
and an output console. Get the separate download from
[Scripta Playground](https://github.com/trafficinc/scripta-playground).
The playground is optional and is not included in normal Scripta installations.

## Releases and platforms

Get platform archives from [release downloads](https://github.com/trafficinc/scripta/releases).
Standard packages include SQLite, reference documentation, and runnable examples.
See [installation and uninstallation](docs/install.md) for checksum verification
and PATH setup, and [compatibility](docs/compatibility.md) for target architectures
and OS baselines.

## What Scripta provides

Scripta focuses on command-line tools, scheduled jobs, server automation, and
report generation. Run scripts directly or through cron and systemd; publish
generated HTML or JSON through an existing web server, or serve dynamic HTTP
requests behind a reverse proxy.

- **Everyday automation:** filesystem operations, literal command arguments,
  bounded process output, timeouts, and local system metrics.
- **Structured data and APIs:** lists, maps, JSON, HTTPS requests, timezone-aware dates,
  and structured local logging.
- **Reusable scripts:** default/variadic parameters, anonymous functions, lexical
  captures, local `require` / `require_once`, and versioned Git/local packages.
- **Developer tools:** native Scripta tests (`scripta test`), syntax checks, a comment-preserving formatter, error codes,
  source locations, and runtime stack traces.
- **Built-in SQLite:** default builds include parameterized SQL,
  transactions, and bounded query results. See the [SQLite extension](docs/sqlite.md).
  Source builds can opt into the [MySQL connector](docs/database-connectors.md)
  with `--features mysql`; verbose version output lists connector versions and capabilities.
  PostgreSQL is available with `--features postgres`; see its [connector guide](docs/postgres.md).
  The [MySQL](examples/mysql.scripta) and [PostgreSQL](examples/postgres.scripta) examples
  use environment credentials and verified TLS.
  Packages can declare [native API/capability requirements](docs/packages.md#native-extension-requirements)
  checked during installation and execution.

Scripta is dynamically typed with strict operations and explicit conversions.
Its syntax is inspired by PHP; the [language specification](docs/language-spec.md)
defines what it supports. Double-quoted strings support `{$expression}` interpolation.
See [objects and classes](docs/objects.md) and
[array expressions](docs/arrays.md) for their language guides.

## Quick start

Save this as `hello.scripta`.

```scr
$name = "Scripta";
echo "Hello, " . $name . "!\n";
```

With the interpreter installed on PATH:

```sh
scripta run hello.scripta
# Hello, Scripta!
```

`scripta hello.scripta` is equivalent. Arguments after the script path are available
through `Cli::args()` and `Cli::arg()`. A `.scripta` file remains source code: run it
through `scripta`; building the interpreter does not compile each script into an app.

### Try the playground apps

From the repository root (the directory containing `Cargo.toml`), build once and
run any of these small apps:

```sh
cargo build -p scr-cli --locked
./target/debug/scripta run playground/expenses.scripta
./target/debug/scripta run playground/table.scripta
./target/debug/scripta run playground/scores.scripta
./target/debug/scripta run playground/arrays.scripta
./target/debug/scripta run playground/strings.scripta
./target/debug/scripta run playground/collections.scripta
./target/debug/scripta run playground/log-analyzer.scripta playground/sample-events.txt
./target/debug/scripta run playground/errors.scripta
./target/debug/scripta run playground/functions.scripta
./target/debug/scripta run playground/reporting.scripta
./target/debug/scripta run playground/streaming.scripta playground/sample-events.txt
```

| Script | What to try |
| --- | --- |
| [expenses.scripta](playground/expenses.scripta) | Edit the expense map; `foreach` prints each item and calculates the total. |
| [table.scripta](playground/table.scripta) | Change `$number`; a `while` loop prints its multiplication table. |
| [scores.scripta](playground/scores.scripta) | Change the scores; `if` / `elseif` / `else` assigns a result to each name. |
| [arrays.scripta](playground/arrays.scripta) | Build a list with `[] =` and assignment at its length, replace an item, and append to a list inside a map. |
| [strings.scripta](playground/strings.scripta) | Parse, normalize, search, and format a Unicode status record with the `String` helpers. |
| [collections.scripta](playground/collections.scripta) | Transform lists, merge and compare maps, aggregate values, and extract record columns. |
| [log-analyzer.scripta](playground/log-analyzer.scripta) | Analyze [sample events](playground/sample-events.txt), count log levels, list sorted services, and report malformed records. |
| [errors.scripta](playground/errors.scripta) | Recover from indexing, arithmetic, and operand-type failures; add `--uncaught` to demonstrate a diagnostic and exit status 1. |
| [streaming.scripta](playground/streaming.scripta) | Stream log records from a file or stdin (`-`) with a line-size limit; file cleanup uses `finally`. |
| [reporting.scripta](playground/reporting.scripta) | Summarize durations with Math helpers and bounded string formatting. |
| [functions.scripta](playground/functions.scripta) | Try defaults, typed variadics, anonymous callbacks, captured counters, and independent collection defaults. |

After editing, check a script with
`./target/debug/scripta check playground/expenses.scripta`, then run it again. There is no
separate compilation step for `.scripta` files. With an installed interpreter, use
`scripta run playground/expenses.scripta` instead.

## Run a standalone executable

Choose the archive for your platform from
[release downloads](https://github.com/trafficinc/scripta/releases), verify its
hash against `SHA256SUMS`, and extract it.
See [installation and uninstallation](docs/install.md) for checksums, PATH setup,
and platform security prompts.

Run these from the extracted package directory. Save the greeting shown above
as `hello.scripta`; examples and documentation remain in the source repository:

| System | Inspect the executable | Run a script |
| --- | --- | --- |
| macOS | `./scripta version --verbose` | `./scripta run hello.scripta` |
| Linux | `./scripta version --verbose` | `./scripta run hello.scripta` |
| Windows (PowerShell) | `.\scripta.exe version --verbose` | `.\scripta.exe run .\hello.scripta` |

You can keep the interpreter anywhere and give it an absolute script path. In
PowerShell, invoke a quoted executable path with `&`, for example:

```powershell
& 'C:\Tools\Scripta\scripta.exe' run 'C:\Scripts\hello.scripta'
```

After placing the executable on PATH, `scripta run hello.scripta` works on all three
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
./target/aarch64-apple-darwin/dist/scripta version --verbose
./target/aarch64-apple-darwin/dist/scripta run examples/hello.scripta
```

macOS support is limited to Apple Silicon (M-series) Macs using
`aarch64-apple-darwin`. Intel Macs are outside the supported target matrix.
Older macOS versions have not been verified.

### Linux

Install GCC or Clang and the development libraries/linker for your distribution,
then install Rust through rustup. On Ubuntu/Debian, the usual linker prerequisite
is `build-essential` (`sudo apt-get install build-essential`).

```sh
cargo test --workspace --locked
cargo build -p scr-cli --profile dist --locked
./target/dist/scripta version --verbose
./target/dist/scripta run examples/hello.scripta
```

Cargo selects the native host architecture: x86-64 on an x86-64 host, ARM64 on an
ARM64 host. Linux ARM64 is source-only; prebuilt Linux releases target x86-64.
Configured Linux distribution candidates use GNU/glibc targets built
on Ubuntu 22.04. musl/Alpine and ARM32 are outside the release matrix.

### Windows

Install Rust through rustup using the **MSVC** toolchain and Visual Studio Build
Tools with the **Desktop development with C++** workload and Windows SDK.

1. Install Rust
Open PowerShell or Windows Terminal and run:

```powershell
winget install --id Rustlang.Rustup -e
```

Close and reopen your terminal, then run:

```powershell
rustup default stable
rustc --version
cargo --version
```

2. Install the Windows C++ build tools
   
Scripta needs the Microsoft Visual C++ compiler and Windows SDK.

Download and install:

Visual Studio Build Tools 

In the installer:

Select Desktop development with C++.

Make sure MSVC build tools are selected.

Make sure a Windows 11 SDK is selected.

Complete the installation.

3. Download Scripta
In PowerShell, run:

```powershell
cd $HOME
git clone https://github.com/trafficinc/scripta.git
cd scripta
```

If Git isn't installed:

```powershell
winget install --id Git.Git -e
```

Restart PowerShell before continuing.

4. Build and install Scripta
From the Scripta repository folder, run:


```powershell
cargo install --path crates/scr-cli --target x86_64-pc-windows-msvc --locked
```

This compiles Scripta and installs its executables into:


```powershell
C:\Users\YOUR_USERNAME\.cargo\bin
```

The installation includes both scripta.exe and scr.exe.

5. Make sure it's on PATH
Rust normally adds its Cargo bin directory to your user PATH automatically.

Check:

```powershell
$env:Path -split ';' | Select-String '\.cargo\\bin'
If nothing appears, add it permanently:
```

```powershell
$cargoBin = "$HOME\.cargo\bin"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")

if (($userPath -split ';') -notcontains $cargoBin) {
    [Environment]::SetEnvironmentVariable(
        "Path",
        "$cargoBin;$userPath",
        "User"
    )
}
```

Close and reopen PowerShell.

6. Verify the installation

Run:

```powershell
scripta version
```

You should be able to execute either command from any directory.

## Or if you don't want to run it from path:

Open PowerShell in the repository root:

```powershell
cargo test --workspace --target x86_64-pc-windows-msvc --locked
cargo build -p scr-cli --profile dist --target x86_64-pc-windows-msvc --locked
.\target\x86_64-pc-windows-msvc\dist\scripta.exe version --verbose
.\target\x86_64-pc-windows-msvc\dist\scripta.exe run .\examples\hello.scripta
```

Use the explicit `--target` on Windows Cargo build/test/install commands. The
project enables the static MSVC CRT for that target; separating host builds keeps
that setting away from compiler plugins. Windows ARM64 is outside the matrix.

For a downloaded Windows executable, add Scripta to your **user PATH** so you can
run it from any folder. Rust and Visual Studio are not required for downloaded
executables.

1. Extract the download and put `scripta.exe` in a permanent folder, such as
   `C:\Users\YourName\Apps\Scripta`. Replace `YourName` with your Windows username.
2. Search Start for **Edit environment variables for your account**.
3. Under **User variables**, select **Path** → **Edit** → **New**.
4. Add the folder containing `scripta.exe`, not the executable itself.
5. Click **OK** on all dialogs, then close and reopen PowerShell.

Verify that the command is available:

```powershell
scripta version --verbose
```

You can then run an existing script with `scripta`, followed by its actual file
path, without typing the path to `scripta.exe`.

### Install a source build on PATH

On macOS/Linux, `cargo install --path crates/scr-cli --locked` installs into
Cargo's bin directory. On Windows use:

```powershell
cargo install --path crates/scr-cli --target x86_64-pc-windows-msvc --locked
```

Restart the terminal if PATH was just updated, then run `scripta version --verbose`.
For a downloaded binary, follow the [per-user installation steps](docs/install.md)
instead. The `dist` profile optimizes and strips the executable while preserving
panic unwinding and overflow checks.

## Linux server automation

Copy or clone the repository onto the Linux server and follow the Linux build
instructions above **on that server**. A macOS executable cannot run on Linux.
After the build passes, install the interpreter system-wide:

```sh
sudo install -m 755 target/dist/scripta /usr/local/bin/scripta
/usr/local/bin/scripta version --verbose
/usr/local/bin/scripta run examples/server-health.scripta /
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
*/5 * * * * cd /opt/scripta && /usr/local/bin/scripta run /opt/scripta/examples/server-health.scripta / >> "$HOME/scripta-logs/health.log" 2>&1
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
| `scripta run script.scripta` | Execute a script; `scripta script.scripta` is shorthand. |
| `scripta run - [arguments...]` | Read script source from stdin through EOF and execute it. |
| `scripta check script.scripta` | Validate source syntax, declarations, and control flow without execution. |
| `scripta test [paths...]` | Run native Scripta tests in isolated workers; see [testing](docs/testing.md). |
| `scripta fmt script.scripta` | Format indentation and spacing while preserving comments and string literals. |
| `scripta fmt --check script.scripta` | Check formatting without writing; useful in CI. |
| `scripta serve app.scripta` | Receive HTTP requests with persistent workers; use a reverse proxy for public HTTPS. |
| `scripta playground` | Launch the separately installed, optional local browser playground. |
| `scripta version` | Print the interpreter version. |
| `scripta version --verbose` | Show version, build target, compiler, profile, commit metadata, and compiled connector versions/capabilities. |
| `scripta help` | Show usage; `scripta help run`, `check`, `fmt`, `test`, or `serve` shows command help. |

`check` is a syntax/declaration check, not complete static type checking. It does
not execute or load required files. `fmt` is idempotent. Use `scripta run -- help`
for a script whose path matches a command name.

Exit codes: **0** for success, **1** for script/file failures or formatting that
needs changes, **2** for CLI usage errors, and **130** for interruption.
A script can explicitly return a code from 0 through 255 with `exit(code)`.

### Shell pipelines

A file-backed script can consume piped data without printing a prompt:

```scr
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
cat playground/sample-events.txt | scripta run playground/streaming.scripta -
printf '%s\n' 'echo "Hello from stdin!\n";' | scripta run -
```

`scripta run -` consumes stdin as UTF-8 source through EOF, subject to
`SCR_MAX_SOURCE_BYTES`. Relative includes and package lookup start from the working
directory; diagnostics identify the source as `<stdin>`. Stdin is then at EOF for
script input. Use a file-backed script when stdin should carry data. A literal
file named `-` remains accessible with `scripta run -- -` or `scripta run ./-`.
`run`, `check`, and `fmt` also accept `--help` or `-h` before the source path.

## Web-server use

Run a dynamic application locally with:

```sh
scripta serve examples/web-app.scripta --host 127.0.0.1 --port 8080
```

Scripta runs a fixed entry script in a fresh runtime for each HTTP request.
Persistent worker processes cache the parsed entry script. Use Caddy or Apache
as an HTTPS reverse proxy for public hosting. `Web` APIs are available during
requests; `Http` remains the outgoing HTTP client.

| Task | Guide and example |
| --- | --- |
| Configure the server | [TOML settings and CLI overrides](docs/web-servers.md#server-configuration); inspect with `--check-config` or redacted `--print-config`. |
| Reload during development | [Opt-in watch mode](docs/web-servers.md#development-watch-mode), coherent source snapshots, and last-good fallback; start with [the watch application](examples/watch/main.scripta). |
| Manage worker lifetime | [Runtime and limits](docs/web-servers.md#runtime-and-limits), automatic recovery, and optional `--worker-max-requests` recycling. Recycling retains cached source. |
| Observe requests and capacity | [Structured logs and private health endpoints](docs/web-servers.md#logs-and-health-checks): `/livez`, `/readyz`, and `/metrics`. |
| Deploy and stop an instance | [Service management and bounded shutdown](docs/web-servers.md#deployment-and-service-management), with [deployment templates](examples/deploy/). |
| Stream output or transfer files | [Incremental responses and flushing](docs/web-servers.md#incremental-responses-and-flushing), [multipart uploads](docs/web-servers.md#multipart-forms-and-uploads), and [raw uploads/downloads](docs/web-servers.md#raw-uploads-and-file-downloads). |

See the [web hosting guide](docs/web-servers.md) for proxy trust, sessions, request
limits, and runnable examples. Keep development watch mode opt-in; use immutable
application releases and instance replacement for deployment.

For periodically generated dashboards, the
[static-output guide](docs/web-servers.md#generate-static-html-or-json-on-any-platform)
remains available on all platforms.

### A dashboard with Caddy or Apache on Linux

For a dashboard that shows periodically refreshed data, have Scripta generate
HTML and JSON, schedule that generation with cron, and let the web server serve
the output. Start with `report.scripta` in the
[static-output guide](docs/web-servers.md#generate-static-html-or-json-on-any-platform).
For example, keep the script at `/opt/scripta/app/report.scripta` and generate its
`public/` output under `/srv/scripta-dashboard`:

```sh
# Create this directory as administrator, then grant the automation user ownership.
sudo mkdir -p /srv/scripta-dashboard/public
sudo chown -R "$(id -un):$(id -gn)" /srv/scripta-dashboard
cd /srv/scripta-dashboard
/usr/local/bin/scripta run /opt/scripta/app/report.scripta
```

Ensure the web-server account can traverse the directory and read the generated
files. Schedule the same `cd` and `scripta run` command in cron to refresh the data.
Use `File::writeAtomic` in the report script so concurrent readers receive a
complete old or new version of each file. New Unix files start private; after the
first run, grant the web server read access to the public reports (for example,
`chmod 644 public/status.json public/index.html`). Later replacements preserve
these ordinary permission bits. Each file is published independently. The sample report is a
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

The [complete reference](docs/standard-library.md) covers CLI and request-context
functions, return values, native objects, limits, and errors.

| Modules | Use |
| --- | --- |
| `File`, `Dir`, `Path` | Read/write text and bytes, stream bounded lines, list, copy, move, and organize files. |
| `Process` | Run literal arguments, capture bounded output, manage child processes. |
| `Env`, `Cli` | Environment values, script arguments/options, terminal detection, prompts, and bounded stdin records with `Cli::readLine`. |
| `System` | Platform, CPU sampling, system memory, disk usage, and host details. |
| `Date`, `Log` | Unix timestamps, UTC/local/named-zone formatting with bundled timezone data, parsing, sleep, and redacted UTC JSON logs. |
| `Clock` | Monotonic elapsed-time measurement with fractional milliseconds, independent of calendar time. |
| `Socket` | TCP listeners/clients, UDP endpoints, half-close, options, and OS readiness polling. |
| `Stream` | Bounded binary file and HTTP streams, connected TCP, and UDP datagrams. |
| Output functions | Direct `echo` output and explicit host/transport flushing with `flush`; HTTP buffering is host-owned. |
| `Web`, `Session` (request context) | Incoming requests, responses, redirects, multipart forms/uploads, cookies, and bounded server-side sessions shared across workers. |
| `Json`, `Http` | JSON conversion, binary HTTP/HTTPS requests, and downloads directly to disk. |
| `Sqlite` | Bundled local databases, parameterized SQL, transactions, and bounded query results. |
| `Password` | Argon2id hashing, verification, and rehash detection with bounded native work. |
| `Html` | Escape UTF-8 text for HTML text nodes and quoted attribute values. |
| `Url`, `Form` | UTF-8 percent/query encoding and form bodies. |
| `Bytes`, `Csv`, `Regex` | Explicit UTF-8 conversion, byte slicing, CSV records and streams, and simple Rust-backed regex. |
| `String` | Grapheme length/slicing for emoji and combining text; trim, split/join, search, replace, take substrings, convert case, and format scalar values with `{}`. |
| `Math` | Type-preserving abs, min/max, floor, ceil, and round (ties away from zero). |
| `List`, `Map` | End operations, splice, chunks, search, aggregation, comparisons, record columns, and map merge/construction helpers. |
| Global functions | Explicit conversions, `type`, `len`, and cancellable `sleep`. |

Format local dates with `Date::format(Date::now(), "%Y-%m-%d %H:%M:%S %:z", "local")`,
or pass an IANA name such as `"America/New_York"`. Omitting the timezone keeps UTC.
Timezone rules ship in the executable; see [dates](docs/standard-library.md#dates)
and the [timezone example](examples/timezones.scripta).

Local script reuse uses `require "lib/helpers.scripta"` or
`require_once "lib/config.scripta"`. Relative paths resolve against the including file;
includes share their caller's current scope. See the [module contract](docs/language-spec.md#local-script-loading).

Shared libraries can be installed with the built-in package manager:

```sh
scripta init
scripta add helpers --path ./lib/helpers
# Git sources can select version tags or a specific revision:
scripta add helpers --git https://example.com/team/helpers.git --version '^1.2'
scripta install --locked
scripta list --tree
scripta outdated
# With a configured package index:
scripta search template
scripta add scripta/template
```

Local and Git-hosted indexes provide package discovery; configure `[registry].index`
in `scripta.toml` or set `SCR_PACKAGE_INDEX`. No public catalog is bundled.

Load an installed package with an alias to keep its names isolated:

```scr
require "helpers" as Helpers;
echo Helpers::greeting();
```

Use `Helpers::functionName()` to call module functions. Aliased packages have their
own functions and variables; different libraries can use identical helper names.
Both aliased `require` and `require_once` share one initialized instance across
aliases of the same canonical entry file. All direct module-scope functions are
callable through the alias; no export declaration is needed. Unaliased includes
retain the existing shared-scope behavior. See [alias semantics](docs/packages.md#package-aliases-and-isolation).

Use `scripta remove helpers` when the project no longer needs that dependency.
Commit `scripta.toml` and `scripta.lock`; ignore `.scripta/`.
See [packages](docs/packages.md) for package metadata, transitive dependencies,
offline installs, and update behavior.

Write package tests as top-level `test_*` functions in `tests/*_test.scripta`,
then run `scripta test`. Built-in assertions and isolated workers require no
Python or test framework package. See [testing](docs/testing.md).

## Examples

Start with [hello.scripta](examples/hello.scripta), [variables.scripta](examples/variables.scripta),
[loops.scripta](examples/loops.scripta), and [functions.scripta](examples/functions.scripta).
The [example guide](examples/README.md) also covers file organizing, log cleanup,
directory backups, server health, HTTP requests, and command execution.
File-changing automation examples preview their work and require `--apply` to
perform changes. The older `organize.scripta` example moves files immediately.

## Limits and diagnostics

Errors include codes, source locations, excerpts, and runtime traces. Diagnostics
mask literal/comment contents and redact known credentials. Execution limits can
be configured for trusted automation; for example, on macOS/Linux:

```sh
SCR_MAX_STEPS=1000000 SCR_MAX_VALUE_BYTES=8388608 SCR_MAX_SCOPES=1024 scripta run automation.scripta
```

In PowerShell set `$env:SCR_MAX_STEPS = '1000000'` before invoking `scripta`.
Source files default to an 8 MiB ceiling and value nesting is bounded at 128.
Optional value/scope budgets are logical limits, not a hard process RSS ceiling.
See [resource behavior](docs/standard-library.md#distribution-and-resource-behavior)
and [platform compatibility](docs/compatibility.md).

## Documentation

[Language specification](docs/language-spec.md) ·
[Standard library](docs/standard-library.md) ·
[Installation and uninstallation](docs/install.md) ·
[Compatibility](docs/compatibility.md) ·
[Architecture](docs/architecture.md) ·
[Development](docs/development.md) ·
[Performance](docs/performance.md)

## License

Scripta is distributed under the [MIT license](LICENSE). Packaged candidates
include dependency licenses and notices.
