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

Use `scripta backup.scripta`, `scripta deploy.scripta`, or
`scripta monitor.scripta`. `.scripta` is the official source extension; `.scta`
is a shorthand. The `scr` command and `.scr` files remain compatible.
New scripts start with `<?scripta`. Legacy `<?scr` remains accepted.

## Try Scripta Playground

Try Scripta in a local browser editor with syntax highlighting, example scripts,
and an output console. Get the separate download from
[Scripta Playground](https://github.com/trafficinc/scripta-playground).
The playground is optional and is not included in normal Scripta installations.

## Project status

**0.1.0 release preparation is implemented. A public release has not been published.**
Local macOS ARM64 validation covers debug and distribution-profile tests,
formatting, Clippy, the Rust 1.85 source check, archive checksums and
reproducibility, and an offline test of the packaged executable, including the
collection playground. See the [local validation record](docs/release-validation.md)
for the tested source, feature variants, results, and limitations.
Native Ubuntu 22.04 x86-64/ARM64 and Windows Server 2022 x86-64 validation also
passed; see the
[Actions validation record](docs/release-validation.md#native-github-actions-validation).

The recorded platform evidence predates the command/extension migration. The
renamed executables and archives need fresh native validation before publication.

| Target | Current validation |
| --- | --- |
| macOS ARM64 | Verified locally on macOS 27.0.1; macOS 15 validation pending |
| Linux x86-64 | Verified natively on Ubuntu 22.04 |
| Linux ARM64 | Verified natively on Ubuntu 22.04 |
| Windows x86-64 | Verified natively on Windows Server 2022 |

Build instructions below describe the configured targets; they do not certify an
untested platform. See the [compatibility matrix](docs/compatibility.md) for OS
baselines and evidence, and the [release checklist](docs/release-checklist.md)
for the remaining release work. Candidates have no publisher signatures or
notarization; the reviewed macOS binaries have linker-generated ad hoc signatures.

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

Save this as `hello.scripta`. Start new scripts, including included files, with
`<?scripta`; legacy `<?scr` is also accepted. A closing tag is optional.

```scr
<?scripta
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

Obtain an archive **only after its target has been verified**, check its SHA-256
sidecar, and extract it. No public download URL is available yet. A local macOS
ARM64 candidate is produced under `dist/` during release preparation.
See [installation and uninstallation](docs/install.md) for checksums, PATH setup,
and platform security prompts.

Run these from the extracted package directory. Each package includes `examples/`:

| System | Inspect the executable | Run a script |
| --- | --- | --- |
| macOS | `./scripta version --verbose` | `./scripta run examples/hello.scripta` |
| Linux | `./scripta version --verbose` | `./scripta run examples/hello.scripta` |
| Windows (PowerShell) | `.\scripta.exe version --verbose` | `.\scripta.exe run .\examples\hello.scripta` |

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
ARM64 host. Configured Linux distribution candidates use GNU/glibc targets built
on Ubuntu 22.04. musl/Alpine and ARM32 are outside the release matrix.

### Windows

Install Rust through rustup using the **MSVC** toolchain and Visual Studio Build
Tools with the **Desktop development with C++** workload and Windows SDK.
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

### Install a source build on PATH

On macOS/Linux, `cargo install --path crates/scr-cli --locked` installs into
Cargo's bin directory. On Windows use:

```powershell
cargo install --path crates/scr-cli --target x86_64-pc-windows-msvc --locked
```

Restart the terminal if PATH was just updated, then run `scripta version --verbose`.
For a downloaded binary, follow the [per-user installation steps](docs/install.md)
instead. The `dist` profile optimizes and strips the executable while preserving
panic unwinding and overflow checks; see [release procedures](docs/releases.md).

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
| `scripta check script.scripta` | Validate tags, complete syntax, declarations, and control flow without execution. |
| `scripta fmt script.scripta` | Format indentation and spacing while preserving comments and string literals. |
| `scripta fmt --check script.scripta` | Check formatting without writing; useful in CI. |
| `scripta playground` | Launch the separately installed, optional local browser playground. |
| `scripta version` | Print the interpreter version. |
| `scripta version --verbose` | Show version, build target, compiler, profile, and supplied commit metadata. |
| `scripta help` | Show usage; `scripta help run`, `check`, or `fmt` shows command help. |

`check` is a syntax/declaration check, not complete static type checking. It does
not execute or load required files. `fmt` is idempotent and keeps an optional
closing tag. Use `scripta run -- help` for a script whose path matches a command name.

Exit codes: **0** for success, **1** for script/file failures or formatting that
needs changes, **2** for CLI usage errors, and **130** for interruption.
A script can explicitly return a code from 0 through 255 with `exit(code)`.

### Shell pipelines

A file-backed script can consume piped data without printing a prompt:

```scr
<?scripta
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
printf '%s\n' '<?scripta echo "Hello from stdin!\n";' | scripta run -
```

`scripta run -` consumes stdin as UTF-8 source through EOF, subject to
`SCR_MAX_SOURCE_BYTES`. Relative includes and package lookup start from the working
directory; diagnostics identify the source as `<stdin>`. Stdin is then at EOF for
script input. Use a file-backed script when stdin should carry data. A literal
file named `-` remains accessible with `scripta run -- -` or `scripta run ./-`.
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

The [complete reference](docs/standard-library.md) documents all **123 functions**,
return values, native objects, limits, and errors.

| Modules | Use |
| --- | --- |
| `File`, `Dir`, `Path` | Read/write text and bytes, stream bounded lines, list, copy, move, and organize files. |
| `Process` | Run literal arguments, capture bounded output, manage child processes. |
| `Env`, `Cli` | Environment values, script arguments, options, and prompts. |
| `System` | Platform, CPU sampling, system memory, disk usage, and host details. |
| `Date`, `Log` | UTC timestamps, parsing, sleep, and redacted local JSON logs. |
| `Json`, `Http` | JSON conversion, binary HTTP/HTTPS requests, and downloads directly to disk. |
| `Bytes`, `Csv`, `Regex` | Explicit UTF-8 conversion, byte slicing, CSV records and streams, and simple Rust-backed regex. |
| `String` | Grapheme length/slicing for emoji and combining text; trim, split/join, search, replace, take substrings, convert case, and format scalar values with `{}`. |
| `Math` | Type-preserving abs, min/max, floor, ceil, and round (ties away from zero). |
| `List`, `Map` | Slice, remove, concatenate, sort, test membership, construct maps, and safely look up keys. |
| Global functions | Explicit conversions, `type`, `len`, and cancellable `sleep`. |

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
```

Load an installed package with an alias to keep its names isolated:

```scr
<?scripta
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
