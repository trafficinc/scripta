# Standard library

The optional [SQLite extension](sqlite.md) registers `Sqlite::open`, `execute`,
`query`, and `close` when the CLI is built with `--features sqlite`. These four
functions are separate from the default standard-library export index below.

The standard library registers functions through the public runtime native API.
Scalar built-ins take exactly one positional argument. Invalid counts, types, and
conversion values raise catchable `TypeError` exceptions with script call spans
and stack traces. There is no implicit conversion in arithmetic, boolean
conditions, string concatenation, or output.

| Function | Accepted inputs and behavior |
| --- | --- |
| `str(value): string` | Null becomes `"null"`; bool becomes `"true"`/`"false"`; int and finite float use decimal display; strings are unchanged. |
| `int(value): int` | Int is unchanged; bool becomes 0/1; a finite integral float within i64 range is converted exactly; strings must parse as signed decimal i64 without whitespace or fractional/exponent notation. |
| `float(value): float` | Finite floats are unchanged; ints convert to f64 (large values may round); bool becomes 0.0/1.0; strings must parse as finite floating-point numbers without whitespace. |
| `bool(value): bool` | Bool is unchanged; numeric 0/1 become false/true; exactly `"false"`/`"true"` become false/true. Other values are errors. |
| `type(value): string` | Returns `null`, `bool`, `int`, `float`, `string`, `list`, `map`, or `callable`. Native objects return their host-defined type name. |
| `len(value): int` | Counts Unicode scalar values in a string (not bytes or grapheme clusters), or entries in a list/map. Other values are errors. |
| `sleep(milliseconds): null` | Requires a nonnegative int duration in milliseconds and the `sleep` capability, allowed by default. Zero returns immediately; long sleeps check cancellation in chunks of at most 10 ms of requested sleep. OS scheduling can delay observation. |

`int(1.5)` raises an error rather than truncating. `bool(2)`, `bool("")`, and
`bool("1")` raise errors rather than applying truthiness. Null is accepted only by
`str`, not by the numeric or bool conversions. Collections, native objects, and
callables are not accepted by any conversion. Non-finite floats are never valid.
Float string display need not retain a decimal point (`str(1.0)` is `"1"`), while
the original numeric types remain distinct for strict equality.

```scr
<?scr
$usage = int("85");
echo "Usage: " . str($usage) . "%\n";
$ratio = float($usage) / 100.0;
if (bool("true")) echo str($ratio) . "\n";
```

Embedders opt in by calling `scr_stdlib::register(&mut runtime)?`. Registration
returns `NativeResult<()>`; duplicate names produce `RegistryError` and preserve
the existing registration. `echo` and `exit` remain language statements. Neither
is registered as a native function; exit returns a code without terminating the
embedding process. `File`, `Dir`, `Path`, `Process`, `Env`, `Cli`, `System`, `Date`, `Log`, `Json`, `Http`, `String`, `Math`, `List`, and `Map` are
registered by default.

## Release 0.1.0 export index

This reference covers all 105 registered functions in the default library, plus
native response/entry properties and local loading statements. No optional plugin,
external language runtime, shell, or network is required to use the local APIs.
Detailed signatures, return types, errors, permissions and limitations follow.

| Module | Public functions | Details |
| --- | --- | --- |
| Globals | `str`, `int`, `float`, `bool`, `type`, `len`, `sleep` | Conversion table above |
| String | `String::trim`, `String::split`, `String::join`, `String::contains`, `String::startsWith`, `String::endsWith`, `String::replace`, `String::substring`, `String::lower`, `String::upper`, `String::format` | [Strings](#strings) |
| Math | `Math::abs`, `Math::min`, `Math::max`, `Math::floor`, `Math::ceil`, `Math::round` | [Math](#math) |
| List | `List::remove`, `List::slice`, `List::concat`, `List::contains`, `List::sort` | [Collections](#collections) |
| Map | `Map::create`, `Map::has`, `Map::get`, `Map::keys`, `Map::values`, `Map::remove` | [Collections](#collections) |
| File | `File::exists`, `File::read`, `File::write`, `File::append`, `File::copy`, `File::move`, `File::delete`, `File::size`, `File::modified`, `File::extension`, `File::openLines`, `File::readLine`, `File::close` | [Filesystem](#filesystem-and-paths) |
| Dir | `Dir::exists`, `Dir::create`, `Dir::createAll`, `Dir::files`, `Dir::directories`, `Dir::remove`, `Dir::current`, `Dir::change` | [Filesystem](#filesystem-and-paths) |
| Path | `Path::join`, `Path::filename`, `Path::extension`, `Path::parent`, `Path::normalize`, `Path::absolute` | [Paths](#filesystem-and-paths) |
| Process | `Process::run`, `Process::spawn`, `Process::shell`, `Process::wait`, `Process::poll`, `Process::kill`, `Process::exists`, `Process::currentId` | [Processes](#processes) |
| Env | `Env::get`, `Env::set`, `Env::has`, `Env::remove`, `Env::all` | [Environment](#environment) |
| Cli | `Cli::args`, `Cli::arg`, `Cli::has`, `Cli::option`, `Cli::prompt`, `Cli::readLine` | [CLI](#script-arguments-and-prompts) |
| System | `System::platform`, `System::architecture`, `System::hostname`, `System::cpuUsage`, `System::memoryUsage`, `System::diskUsage`, `System::uptime`, `System::homeDirectory`, `System::tempDirectory` | [System](#system-information) |
| Date | `Date::now`, `Date::format`, `Date::parse`, `Date::sleep` | [Dates](#dates) |
| Log | `Log::info`, `Log::warn`, `Log::error`, `Log::debug`, `Log::setFile` | [Logging](#local-logging) |
| Json | `Json::encode`, `Json::decode`, `Json::pretty` | [JSON](#json) |
| Http | `Http::get`, `Http::post`, `Http::request` | [HTTP](#http) |

`DirEntry`, `ProcessResult`, `ProcessHandle`, `MemoryUsage`, `DiskUsage`,
`HttpResponse`, and `FileStream` are read-only native objects whose properties
and methods are listed in their module sections. Convert selected properties into ordinary maps before
JSON encoding. `require` and `require_once` are statements, not native functions;
see [local script loading](language-spec.md#local-script-loading). The built-in
[package manager](packages.md) installs declared Git and local dependencies;
`require` and `require_once` can resolve installed packages by name. Script loading
does not implicitly fetch remote modules.

Default and variadic parameters and anonymous literals are language features,
separate from native function registration. A script variadic annotation checks
each extra argument and binds a list. See
[function parameters and captures](language-spec.md#function-parameters-and-captures)
and [the functions playground](../playground/functions.scr). Callback-based
collection transforms remain deferred; callbacks can be passed to script helpers.

## Strings

All helpers return new values and leave their arguments unchanged. Arguments
have the exact types below. Only `format` explicitly accepts scalar values for
string display; other helpers do not coerce scalar values to strings.
Wrong argument counts or types raise catchable `TypeError` exceptions.

| Function | Behavior |
| --- | --- |
| `String::format(template: string, values: list): string` | Replaces sequential `{}` placeholders with scalar display. `{{` and `}}` emit literal braces. See the formatting contract below. |
| `String::trim(text: string): string` | Removes Unicode whitespace from both ends, preserving interior whitespace. Empty or all-whitespace input returns `""`. |
| `String::split(text: string, separator: string): list` | Splits on literal, nonoverlapping separators and preserves leading, trailing, and consecutive empty fields. Empty text returns `[""]`; no match returns `[text]`. An empty separator raises `RangeError`. |
| `String::join(items: list, separator: string): string` | Joins a list containing only strings. Empty input returns `""`, one item returns that item, and an empty separator concatenates items. Non-string elements raise `TypeError`. |
| `String::contains(text: string, search: string): bool` | True when the literal search occurs anywhere in text. An empty search returns true, including in empty text. |
| `String::startsWith(text: string, search: string): bool` | True when text begins with the literal search. An empty search returns true. |
| `String::endsWith(text: string, search: string): bool` | True when text ends with the literal search. An empty search returns true. |
| `String::replace(text: string, search: string, replacement: string): string` | Replaces all literal, nonoverlapping matches from left to right. Replacement text is not searched again. No match returns the original text's value; an empty replacement deletes matches. An empty search raises `RangeError`. |
| `String::substring(text: string, start: int, length: int optional): string` | Takes nonnegative Unicode scalar positions. Omitted length returns the suffix. Start beyond `len(text)`, negative offsets, or length beyond the remaining text raise `RangeError`. Start at the end is valid with omitted length or length zero. Empty text accepts start zero with omitted length or length zero. Explicit null is not an omitted length. |
| `String::lower(text: string): string` | Applies locale-independent Unicode lowercase, including contextual Greek final sigma. Empty input returns `""`. Mappings can expand a scalar, such as `İ` becoming `i` plus a combining dot. |
| `String::upper(text: string): string` | Applies locale-independent Unicode uppercase. Empty input returns `""`. Mappings can expand scalars, such as `ß` becoming `SS`. |

`String::format` accepts exactly one value per `{}` placeholder. Values may be
null, bool, int, finite float, or string, with the same display as `str` (so 1.0
prints as `1`). Replacement strings are inserted literally and never reparsed.
For example, `String::format("{{{}}}: {}", ["café", 2])` returns `"{café}: 2"`.
Empty templates with empty value lists return `""`. Unmatched braces, unsupported
placeholder syntax, and missing or extra values raise `FormatError`; unsupported
value types raise `TypeError`. There are no width, precision, positional, or named
placeholders. Unicode is preserved without normalization. Output is preflighted
before allocating the result and uses the byte budget described below.

Matching is case-sensitive and literal, with no regular expressions or Unicode
normalization. For example, precomposed `é` and `e` followed by a combining acute
accent are different searches. Substring positions use the same Unicode scalar
unit as `len`, not bytes or grapheme clusters; a combining mark can be selected
separately. Trimming and case conversion use Rust's Unicode tables, independently
of the operating system locale.

```scr
<?scr
$parts = String::split(String::trim("  café,tea,,  "), ",");
echo Json::encode($parts) . "\n"; // ["café","tea","",""]
echo String::join(["café", "tea"], " / ") . "\n"; // café / tea
echo str(String::contains("café", "fé")) . "\n"; // true
echo str(String::startsWith("café", "ca")) . "\n"; // true
echo str(String::endsWith("café", "fé")) . "\n"; // true
echo String::replace("a-a", "a", "b") . "\n"; // b-b
echo String::substring("é🦀abc", 1, 2) . "\n"; // 🦀a
echo String::lower("HELLO") . "\n"; // hello
echo String::upper("Straße") . "\n"; // STRASSE
```

The helpers need no OS capability, external command, or network access. String
results account for one Value slot plus UTF-8 output bytes. Split results account
for a list Value slot plus one Value slot and UTF-8 bytes for each field, including
empty fields. Configured `max_value_bytes` limits are checked before reserving
the result buffer or adding a split field. Case conversion preflights expanded
UTF-8 length before constructing the Unicode result. Detected size overflows,
budget violations, and fallible reservation failures raise catchable
`ResourceLimitError`. These are logical per-value limits, not a hard process
memory ceiling; input arguments are still subject to the runtime's ordinary
limits. Cancellation is checked at native-call boundaries and during iterative
result construction. A failed helper does not replace an assignment destination.

Try [the strings playground](../playground/strings.scr) for a complete status
record normalization example.

## Math

These pure helpers require no OS permissions. They accept only ints and finite
floats, preserving the input type and requiring explicit conversions when mixing
numeric types. Invalid types, non-finite values, and wrong arity raise `TypeError`.

| Function | Behavior |
| --- | --- |
| `Math::abs(value: int or float): int or float` | Absolute value. i64 minimum raises `ArithmeticError` because its positive value cannot fit in i64. Float negative zero becomes positive zero. |
| `Math::min(left, right): int or float` | Smaller of two ints or two finite floats. Equal values select the left argument, including signed zero. |
| `Math::max(left, right): int or float` | Larger of two ints or two finite floats. Equal values select the left argument, including signed zero. |
| `Math::floor(value: int or float): int or float` | Rounds toward negative infinity; ints are unchanged. `Math::floor(-1.2)` returns float -2.0. |
| `Math::ceil(value: int or float): int or float` | Rounds toward positive infinity; ints are unchanged. `Math::ceil(-1.2)` returns float -1.0. |
| `Math::round(value: int or float): int or float` | Nearest integer, with halfway ties away from zero: 2.5 becomes 3.0 and -2.5 becomes -3.0. Ints are unchanged. |

Float rounding returns a float even outside i64 range and preserves signed zero
where applicable. Use `int` explicitly for conversion, which still rejects
out-of-range values. Rounding operates on the represented binary floating-point
value. No decimal precision argument, power, or random API is provided.

Try [the reporting playground](../playground/reporting.scr):
`cargo run -p scr-cli --locked -- run playground/reporting.scr`.

## Collections

List and Map helpers return new values and leave input collections unchanged.
Nested lists and maps are copied; native object and callable handles retain
their identity, matching ordinary assignment and argument passing. List results
have contiguous indexes starting at zero. Map results and key/value projections
preserve insertion order and distinguish integer keys from string keys.

| Function | Behavior |
| --- | --- |
| `List::remove(items: list, index: int): list` | Copies the list without the existing zero-based index and renumbers remaining positions. Negative or absent positions raise `RangeError`, including removal from an empty list. |
| `List::slice(items: list, start: int, length: int optional): list` | Copies a contiguous range using nonnegative positions. Omitted length copies the suffix. Start beyond `len(items)`, negative offsets, or length beyond the remainder raise `RangeError`. Start at the end with omitted or zero length returns `[]`; `List::slice([], 0)` returns `[]`. Explicit null is not an omitted length. |
| `List::concat(left: list, right: list): list` | Returns left's elements followed by right's. Empty lists are valid. Both inputs remain unchanged. |
| `List::contains(items: list, value: any): bool` | Tests membership with strict recursive equality, like `===`. Int `1`, float `1.0`, and string `"1"` differ. Lists compare in order; maps compare contents independent of insertion order. Native objects and callables compare by identity. Empty lists return false. |
| `List::sort(items: list, descending: bool optional): list` | Returns a stable sorted copy, ascending by default. Nonempty lists must contain only ints, only finite floats, or only strings. Mixed types, bools, nulls, collections, objects, and callables raise `TypeError`, even in a single-element list. Empty lists are valid. Set descending to true to reverse the comparison direction while preserving equal elements' original order. |
| `Map::create(): map` | Returns an empty map. `[]` continues to create an empty list. Add map entries with normal indexed assignment. |
| `Map::has(items: map, key: int or string): bool` | Tests whether the exact typed key exists, including keys whose value is null. |
| `Map::get(items: map, key: int or string, default: any optional): any` | Returns a copy of an existing value, or the default if the key is absent. Omitted default is null. A stored null is returned as null, not replaced by the default. As in other calls, default arguments are evaluated eagerly even when the key exists. |
| `Map::keys(items: map): list` | Returns the map's keys in insertion order, preserving each int/string type. An empty map returns `[]`. |
| `Map::values(items: map): list` | Returns copies of values in insertion order. An empty map returns `[]`. |
| `Map::remove(items: map, key: int or string): map` | Returns a copy without that key, preserving the remaining entries' order. An absent key returns an unchanged copy, including for an empty map. |

Arguments are strictly typed; invalid types and argument counts raise catchable
`TypeError`. No helper converts a list into a map, coerces a key, creates holes,
or creates missing intermediate collections. Sorting uses the language's numeric
and UTF-8 string ordering, with no locale collation or numeric coercion. Stable
float sorting preserves the order of equal values, including positive and
negative zero.

```scr
<?scr
$items = [3, 1, 2];
echo Json::encode(List::sort($items)) . "\n"; // [1,2,3]
echo Json::encode(List::remove($items, 1)) . "\n"; // [3,2]
echo Json::encode(List::slice($items, 1, 2)) . "\n"; // [1,2]
echo Json::encode(List::concat($items, [4])) . "\n"; // [3,1,2,4]
echo str(List::contains($items, 2)) . "\n"; // true
$config = Map::create();
$config["name"] = "worker";
$config["optional"] = null;
echo str(Map::has($config, "optional")) . "\n"; // true
echo str(Map::get($config, "optional", "fallback")) . "\n"; // null
echo str(Map::get($config, "retries", 3)) . "\n"; // 3
echo Json::encode(Map::keys($config)) . "\n"; // ["name","optional"]
echo Json::encode(Map::values(Map::remove($config, "optional"))) . "\n"; // ["worker"]
```

Helpers need no OS capability, network access, or external command. Before
copying result collections, they check the 128-level nesting limit and optional
`max_value_bytes` budget using the runtime's logical accounting: one Value slot
per value, UTF-8 string bytes, and MapKey slots plus string-key bytes. Detected
size overflows, byte-budget violations, and fallible result reservations raise
`ResourceLimitError`; input arguments and returned booleans also remain subject
to the runtime's ordinary value checks. These limits do not bound process RSS
or sorting scratch space. Cancellation is checked at native-call boundaries
and during result traversal/copying; a sort can delay cancellation while it is
ordering the result. Failed helpers leave assignment destinations unchanged.

`Map::get` and `has` provide safe lookups without exceptions. Direct missing-key
indexing and invalid list positions raise catchable core `IndexError`; invalid
core operand/index types raise `TypeError`, and arithmetic overflow or division
by zero raises `ArithmeticError`. Core runtime safety limits and cancellation
remain outside catches. Existing helper `RangeError` and `ResourceLimitError`
exceptions are unchanged. See the [exception contract](language-spec.md#statement-grammar)
and [errors playground](../playground/errors.scr). Callback transforms and pop
are deferred. Try the [collections playground](../playground/collections.scr)
and [log analyzer](../playground/log-analyzer.scr) for complete examples.

## Registering Rust modules

For a complete extension crate with optional Cargo features, CLI startup wiring,
tests, and distribution instructions, see [Writing built-in extensions](writing-extensions.md).

Dependencies point from `scr-stdlib` to `scr-runtime`, then from `scr-runtime` to
`scr-ast`/`scr-diagnostics`. The runtime does not depend on the standard library.
A future module exports a registration function using only the public runtime API.
For example, a separately registered host extension can expose an environment value:

```rust
use scr_runtime::{NativeError, NativeFunction, NativeResult, NativeSignature,
                  NativeType, Runtime, Value};

pub fn register_host(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register("Host::env", NativeFunction::new(
        NativeSignature::new(vec![NativeType::String], NativeType::String),
        |context, args| {
            let [Value::String(name)] = args else {
                return Err(NativeError::type_error("Host::env() requires one string"));
            };
            let value = context.env(name)?.unwrap_or_default().to_str()
                .ok_or_else(|| NativeError::new("EncodingError", "environment value is not UTF-8"))?;
            Ok(Value::String(value.into()))
        },
    ))
}
```

This is a host-extension example using the same environment snapshot as `Env`;
it requires the `environment.read` capability. No interpreter edits
are needed: add the module and invoke its registration from `scr_stdlib::register`
or the embedding host. `register_native` is an alias of `register`. Registration
accepts a bare identifier or one `Namespace::method` pair; reserved keywords,
invalid names, and conflicts with existing native/script functions are rejected.
Registrations are sequential; a module should propagate the first error.

`NativeFunction` holds a required `NativeSignature` and a Rust closure implementing
`Fn(&mut RuntimeContext<'_>, &[Value]) -> NativeResult<Value>`. Closures can capture
host state. Signatures specify exact positional arity, argument constraints, and
the return constraint. `NativeSignature::with_variadic(type)` validates trailing
positional arguments; `Path::join` uses this with no required parameters.
`NativeType::Any` and `NativeType::OneOf` support dynamic
and union inputs. Objects use `NativeType::Object("HostType")`. Counts and types
are checked before Rust code runs, with no implicit conversions. Return types,
finite floats, and collection integrity are checked after the callback. Bad host
return values raise `NativeError` exceptions. Use checked operations and pattern
matching when validating argument contents; avoid indexing/unwrap on script data.

This replaces the old bare `fn(&[Value]) -> Result<Value, String>` registration:
wrap callbacks with a signature and accept the context argument. Both runtime
registration and standard-library registration now return results.

## Runtime context and errors

Set `RuntimeOptions.context: ContextOptions` before `Runtime::with_options`.
The CLI supplies the script path and only script arguments, excluding the
executable, interpreter command, and script path. Embedders can supply arguments
and metadata directly. The context
contains:

| Field | Host behavior |
| --- | --- |
| `working_directory` | Mutable runtime-local `PathBuf`; defaults to the process working directory. `Dir::change` changes only this runtime. |
| `script_path` | Optional host-selected source path. |
| `cli_arguments` | OS strings preserve native argument encoding. |
| `environment` | A mutable runtime-local snapshot of OS strings; defaults to the host environment. Env writes affect subsequent children. `context.env(name)` checks `environment.read`. |
| `input` | Defaults to buffered process stdin; override with `Runtime::set_input_stream` for prompts. |
| `output` | The same writer passed to `Runtime::execute`, shared with echo and prompts. |
| `error` | Defaults to process stderr; override with `Runtime::set_error_stream`. |
| `cancellation` | The atomic cancellation flag from runtime options; `check_cancelled()` returns an interruption error. |
| `permissions` | Automation capabilities are allowed by default: sleep, filesystem read/write, environment read/write, process execute/inspect, system.inspect, log.write, http.request, cli.read, and cli.prompt. `process.shell` remains opt-in. |
| `call_span` | Original script span of the native call. |
| `max_value_bytes` | Optional host logical-value budget; file reads enforce it while buffering, and String helpers check result sizes before allocating output. |

Capabilities are a cooperation interface for trusted Rust implementations, not a
sandbox around arbitrary host code. Hosts grant capabilities by inserting names
into `PermissionConfig.allowed`. Modules enforce them; registering additional
OS APIs never requires changing the interpreter. Environment snapshots do not
mutate the process environment.

Callbacks return `NativeError::new("ExceptionName", message)` or use conversions:
`std::io::Error` converts via `?` into `IoError`; a Rust `String` converts into
`NativeError`. The runtime attaches the script call span and active stack frames.
Script catches match the exception name, `Error`, or an untyped catch. The catch
binding contains the message string, but a `catch (string $e)` does not match a
native exception simply because the message is a string. Uncaught exceptions
reach the host as `RuntimeError` with kind-prefixed messages and original traces.
Cancellation uses the reserved `Interrupted` kind and bypasses script catches.
Blocking native modules must cooperate with cancellation to make timely progress.

Native return values can include `Value::NativeObject(Rc<dyn NativeObject>)`.
Implement `type_name` and `get_property`; the default `set_property` rejects writes.
Direct, compound, and indexed property writes all respect this setter. Getter and
setter failures become catchable `PropertyError` exceptions at the property access.
Hosts may opt into writable properties by implementing the setter; no script
classes or inheritance are introduced.


## Filesystem and paths

All filesystem paths and text contents use UTF-8 script strings. Rust's standard
filesystem APIs interpret paths using the running OS: POSIX conventions on Linux
and macOS, drive letters/UNC paths and native separators on Windows. Backslashes
are ordinary filename characters on Unix. Windows drive-relative (`C:foo`) and
rooted-without-drive (`\foo`) inputs are rejected with `PathError`; use ordinary
relative paths or fully absolute paths. Windows canonical paths may use the
verbatim `\\?\` prefix. No tilde, environment-variable, or glob expansion occurs.
NUL characters are rejected. Empty filesystem paths and `Path::absolute("")` are
rejected; empty lexical path components are permitted.

Relative paths resolve against `RuntimeOptions.context.working_directory`, which
starts at the host process working directory, not the script's parent directory.
An injected relative base is resolved against the process working directory.
`Dir::change` updates the runtime-local base for later calls, including calls
inside functions and later execute calls. It never calls `std::env::set_current_dir`
and does not affect other runtimes. Existing `DirEntry` snapshots keep their paths.

| File method | Behavior |
| --- | --- |
| `File::exists(path): bool` | True for a regular file. Missing paths, directories, and final symlinks return false. Permission and other inspection failures raise exceptions. |
| `File::read(path): string` | Reads a complete regular file as UTF-8. Invalid file bytes raise `EncodingError`. |
| `File::write(path, contents): null` | Creates or truncates a regular file, replacing its contents. Parent directories must exist. |
| `File::append(path, contents): null` | Creates a missing file or appends UTF-8 bytes to a regular file. |
| `File::copy(source, destination): null` | Copies bytes, including binary files, to a new file. Refuses any existing destination, including symlinks and directories. Does not copy metadata/permissions. |
| `File::move(source, destination): null` | Moves a regular file to a new name, refusing any existing destination. Uses exclusive hard-link creation followed by source removal; cross-device/unsupported-link errors fall back to exclusive copy then removal. |
| `File::delete(path): null` | Deletes one regular file. Missing paths raise an exception. Never deletes directories or symlinks. |
| `File::size(path): int` | Regular-file size in bytes, checked against i64 range. |
| `File::modified(path): int` | Modification time in whole Unix seconds (floored), checked against i64 range. |
| `File::openLines(path: string, maxLineBytes: int optional): FileStream` | Opens a UTF-8 line stream; default raw line limit is 1 MiB. See [bounded file streams](#bounded-file-streams). |
| `File::readLine(stream: FileStream): string or null` | Returns the next line without LF/CRLF, or null at EOF. |
| `File::close(stream: FileStream): null` | Idempotently closes the stream and all its aliases. |
| `File::extension(path): string` | Lexical suffix without the dot, or empty string. Does not require the file to exist; same as `Path::extension`. |

Copy/move destinations are protected with exclusive OS creation, so a destination
that appears after the initial check is not overwritten. If source removal fails
after move creates its destination, both names remain and an exception states
that the destination remains. Hard-link moves retain metadata; copy fallback uses
new-file metadata. Writes/appends are not atomic replacements and may leave partial
contents on cancellation or I/O failure. Copy failure attempts to delete its newly
created partial destination and reports any cleanup error. Reads, writes, copies,
and listings check cancellation between chunks or entries.

| Directory method | Behavior |
| --- | --- |
| `Dir::exists(path): bool` | True for a directory; false for missing paths, files, and final symlinks. Other errors propagate. |
| `Dir::create(path): null` | Creates one new directory; its parent must exist. Any existing name raises an error. |
| `Dir::createAll(path): null` | Creates missing parents; an existing directory succeeds. Files and final symlinks are rejected. |
| `Dir::files(path): list` | Shallow list of regular-file `DirEntry` snapshots, sorted by UTF-8 name in case-sensitive lexical order. |
| `Dir::directories(path): list` | Shallow list of directory snapshots in the same order. |
| `Dir::remove(path): null` | Removes only an empty directory. Rejects root, current directory, and its ancestors. There is no recursive-delete argument or API. |
| `Dir::current(): string` | Canonical absolute runtime working directory. |
| `Dir::change(path): null` | Requires an existing directory and saves its canonical absolute path in this runtime. |

Listings exclude symlinks and special files and never recurse. All filesystem
operations reject a final symlink except existence checks (false) and lexical
extension extraction. Intermediate symlink directories use OS resolution; the
optional root policy checks their canonical destinations. Errors reading metadata,
permissions, or directory entries propagate instead of silently skipping entries.
If a selected entry's name/path cannot be represented in UTF-8 (including unpaired
Windows surrogates), the entire listing raises `EncodingError`; names are never
lossily replaced. File contents have the same strict UTF-8 rule for `File::read`.

Entries have type `DirEntry` and read-only properties: `name` (basename), `path`
(absolute path), `extension` (suffix or empty string), `size` (bytes for files,
null for directories), `modified` (Unix seconds), and `isDirectory` (bool).
Properties are metadata snapshots; later filesystem changes do not update them.
Direct, compound, and indexed property assignments raise `PropertyError`.

| Path method | Behavior |
| --- | --- |
| `Path::join(...strings): string` | Joins zero or more native path components. An absolute later component replaces the previous path, following Rust path rules. Zero arguments returns empty string. |
| `Path::filename(path): string` | Final filename component, or empty string if none. |
| `Path::extension(path): string` | Last extension without dot. `.hidden` has no extension; `a.tar.gz` gives `gz`. |
| `Path::parent(path): string` | Lexical parent, or empty string if none (including a bare filename or root). |
| `Path::normalize(path): string` | Lexically removes `.` and collapses `..`, preserving leading relative `..` and stopping at absolute roots. Empty input normalizes to `.`. |
| `Path::absolute(path): string` | Resolves against the runtime directory, then lexically normalizes. The target need not exist. |

Path helpers and `File::extension` do no filesystem I/O and require no filesystem
capability. Lexical normalization does not resolve symlinks; applying it before
I/O can change the meaning of paths containing symlinks and `..`. Actual I/O
preserves native path resolution.

### Bounded file streams

`File::openLines(path, maxLineBytes)` opens a regular file using the same read
capability, policy roots, UTF-8 path rules, and final-symlink rejection as
`File::read`. The optional maximum is a strictly positive int, defaulting to
1048576 bytes (1 MiB). It counts each raw line including its LF or CRLF terminator.
Wrong arities/types raise `TypeError`, and nonpositive maxima raise `RangeError`.

`File::readLine(stream)` returns a UTF-8 string, removing a final LF and one CR
immediately before it. Bare CRs and a UTF-8 BOM are preserved. Blank lines return
`""`; empty files and EOF return null, including repeated reads at EOF. A final
unterminated line is returned normally. Invalid UTF-8 raises `EncodingError`.
Read errors use the filesystem error categories below. Oversized lines, output
budget violations, and fallible line-buffer allocation failures raise
`ResourceLimitError`. A read failure closes the stream; it never returns partial
line data. Permission and argument failures before reading leave it unchanged.

File streams have shared native identity, a shared cursor, and a read-only bool
property `closed`. Assignment and collection copies retain the same handle.
`File::close(stream)` drops the descriptor immediately for all aliases; repeated
close calls succeed. Close requires no read permission, although native-call
cancellation still applies. Reading a closed stream raises `StreamError`.
Streams release automatically when their last reference is dropped, including
runtime teardown unless the host retains aliases. Streams are not JSON values;
serialize selected scalar results instead.

Each stream belongs to its filesystem registration. Reading checks the runtime's
`filesystem.read` permission and the registration's read policy. A stream passed
by a host to another registration raises `TypeError` on read; closing a real
stream remains available without read permission. Reads continue on the original
open descriptor after renames or path replacement and do not resolve the path
again. The root check applies at opening. Concurrent path replacement has the
same documented limitations as other filesystem operations.

Reads use an 8 KiB input buffer and retain only the current line. The logical
result budget is one Value slot plus the UTF-8 payload after removing LF/CRLF;
at most two extra terminator bytes are permitted during assembly. This bounds
line assembly by both `maxLineBytes` and the configured value budget, independently
of total file size. Fixed native buffers, allocator overhead, and arguments are
outside the logical per-value limit. Cancellation is checked between buffered
chunks and before returning a line; an interrupted active read closes the stream.
OS reads remain synchronous, so cancellation cannot interrupt a blocked OS call.

```scr
<?scr
$stream = File::openLines("events.txt", 65536);
try {
    while (($line = File::readLine($stream)) !== null) {
        echo $line . "\n";
    }
} finally {
    File::close($stream);
}
```

Finally runs for normal completion and recoverable failures/control flow, under
the ordinary execution limits. Fatal runtime failures and cancellation bypass
script cleanup; dropping the last stream reference still closes its descriptor.
Whole-file helpers remain available. There is no writable stream, iterator syntax,
or asynchronous I/O. The [streaming playground](../playground/streaming.scr)
counts sample log records with bounded per-line input and constant-size counters:
`cargo run -p scr-cli --locked -- run playground/streaming.scr playground/sample-events.txt`.

### Permissions and exceptions

Filesystem operations require `filesystem.read` for observations (including
`Dir::change`) or `filesystem.write` for mutations. Copy requires read permission
on its source and write permission on its destination; move requires write on both.
These capabilities are allowed by default; hosts can remove them from
`PermissionConfig.allowed` to deny access.

Embedders can additionally restrict read/write access and allowed directory roots:

```rust
use scr_stdlib::{FilesystemPolicy, register_with_filesystem_policy};
let policy = FilesystemPolicy {
    read: true,
    write: false,
    roots: Some(vec![std::fs::canonicalize("/data/input")?]),
};
register_with_filesystem_policy(&mut runtime, policy)?;
```

Use `scr_stdlib::register` for unrestricted roots, or the policy registration
instead of it (not both). Roots must be existing absolute directories and are
canonicalized at registration. `roots: None` permits all paths; `Some(vec![])`
denies all paths. Checks resolve symlinks and existing ancestors before checking
component-based containment, including paths with missing suffixes that may be
created. Relative traversal and stable symlinks escaping the roots are denied.
These standard-library checks are not a race-proof OS sandbox: directory trees
must be trusted against concurrent path/symlink replacement. Trusted Rust modules
can access host APIs directly.

Exceptions include `TypeError` (arity/type mismatch), `PathError` (invalid paths),
`FileNotFoundError`, `AlreadyExistsError`, `PermissionError`, `EncodingError`,
`RangeError`, and `FilesystemError` for other I/O or wrong-kind failures. Messages
include the operation and offending path where available; native call spans and
stack traces are retained. Only a genuine not-found inspection becomes false in
an existence check. A missing parent or file/directory collision never causes
implicit deletion or replacement of a different kind of object.

```scr
<?scr
$source = "downloads";
$destination = "documents";
Dir::createAll($destination);
foreach (Dir::files($source) as $file) {
    if ($file->extension === "pdf") {
        File::move($file->path, Path::join($destination, $file->name));
    }
}
echo "Finished organizing files.\n";
```


## Processes

`Process::run(command, arguments = [], options = ...)` launches one executable
with literal positional arguments using Rust's `Command`. The command is one
program name/path, not a shell command line. Spaces, quotes, pipes, redirects,
globs, and `$()` in arguments remain literal. Arguments must be a list of strings;
NUL characters and empty program names raise `TypeError` before launch. A bare
program uses native executable/PATH lookup. Relative executable paths containing
components such as `./tool` resolve against the requested child working directory.

```scr
<?scr
$result = Process::run("git", ["status", "--short"]);
if ($result->success) {
    echo $result->stdout;
} else {
    echo "Git failed: " . $result->stderr;
    exit(1);
}
```

A normal process exit returns a read-only `ProcessResult` object:

| Property | Value |
| --- | --- |
| `exitCode` | Native exit code, or null when Unix signal termination supplies no code. Windows follows Rust's signed exit-code representation. |
| `stdout` | Captured UTF-8 text, or empty string for inherited/null output. |
| `stderr` | Captured UTF-8 text, or empty string for inherited/null output. |
| `success` | Whether the native status is successful (normally exit code zero). |
| `durationMs` | Nonnegative elapsed milliseconds from launch through cleanup/capture. |

A nonzero exit code is a result, not an exception. Spawn failures, timeout,
cancellation, output-limit violations, and invalid UTF-8 output are errors.

### Child options and output

Supply an optional third map, for example:

```scr
$result = Process::run("git", ["status"], [
    "cwd" => "repository",
    "env" => ["LANG" => "C", "UNWANTED" => null],
    "timeoutMs" => 5000,
    "maxOutputBytes" => 1048576,
    "stdout" => "capture",
    "stderr" => "inherit"
]);
```

| Option | Default and behavior |
| --- | --- |
| `cwd` | Runtime working directory. Relative overrides resolve against that base. Does not change parent/runtime cwd. |
| `env` | Overrides the runtime environment snapshot. String values replace/add; null removes a key for this child. An empty list is accepted as an empty override map. |
| `timeoutMs` | 60000 ms by default; nonnegative int, or null to disable the deadline. Zero is an immediate deadline. |
| `maxOutputBytes` | 8388608 bytes (8 MiB) combined across captured stdout/stderr. Per-call limits may lower but cannot exceed the host ceiling. Zero permits no captured bytes. |
| `stdout`, `stderr` | Each accepts `capture`, `inherit`, or `null`. Capture is the default. |
| `stdin` | `null` (immediate EOF) by default; `inherit` uses OS stdin. |

Unknown options and invalid values raise `TypeError` before a child is launched.
Inheritance uses the process's actual OS standard handles, not injected runtime
writers/readers. Corresponding parent writers are flushed before launch to keep
earlier output ordered. Spawned children can interleave output with later script
output. Captured streams retain their individual byte order; there is no combined
stdout/stderr ordering guarantee.

Capture uses separate temporary files, with no stdout/stderr pipe dependency or
reader thread waiting for EOF. A monitor polls status, deadline, cancellation, and
combined file length at roughly 5 ms intervals. Temporary disk use can exceed the
limit between polls; returned capture memory is bounded by the configured limit.
Large/continuous writers are stopped with `OutputLimitError`. Temporary files are
released after capture/cleanup. Disk errors propagate. Capture does not wait for
EOF from descendants that inherited a stream. Invalid UTF-8 raises `EncodingError`
after child cleanup; binary output has no implicit lossy conversion.

Hosts configure registration defaults and capture ceilings through:

```rust
use scr_stdlib::{StdlibOptions, process::ProcessConfig};
use std::time::Duration;
let options = StdlibOptions {
    process: ProcessConfig {
        default_timeout: Some(Duration::from_secs(120)),
        max_output_bytes: 4 * 1024 * 1024,
    },
    ..Default::default()
};
scr_stdlib::register_with_options(&mut runtime, options)?;
```

Use this instead of another standard-library registration. The default timeout
is a default, not a maximum: a call can choose a longer timeout or null. The output
ceiling is enforced per child. Allocation limits for arbitrary script values and
aggregate limits across many simultaneous children remain separate concerns.

### Spawn, handles, and cleanup

`Process::spawn(command, arguments = [], options = ...)` accepts the same options
and returns a read-only `ProcessHandle` immediately. Each handle exposes `pid` and
`running` (whether its monitor has not yet completed). Handles cannot be forged
from integers or a native object that only claims the same type name.

| Method | Behavior |
| --- | --- |
| `Process::wait(handle)` | Waits and returns a ProcessResult, or raises the stored launch-monitor error. Repeated waits retain the completed result. |
| `Process::poll(handle)` | Returns null while pending; otherwise returns the result or raises the stored error. |
| `Process::kill(handle)` | Requests forced termination, waits for cleanup, and returns the resulting status. A completed handle retains its original result. |
| `Process::exists(pid): bool` | Requires a positive representable PID and reports whether it appears in a fresh process-inspection snapshot. OS visibility restrictions can make another user's process invisible. |
| `Process::currentId(): int` | Current Scripta host process ID. |

A background monitor enforces timeouts, capture limits, and cancellation even
while the script does other work. Dropping the last reference to a handle kills
and reaps its owned child. A runtime retains globals until dropped or replaced;
embedding hosts that reuse a runtime may deliberately retain handles across
execute calls. The CLI drops its runtime on success, script exit, and errors.
There is no detach operation. Spawning an unused handle immediately drops it.
Launch errors point to run/spawn; deferred errors point to wait/poll/kill where
observed. Cancellation bypasses script catches.

On Unix, children start in a fresh process group and cleanup sends SIGKILL to
that group plus the direct child, then reaps the direct child. A program that
explicitly detaches descendants into another session/group can escape group
cleanup; process groups are not a security sandbox. On Windows, children are
assigned to a Job Object before being resumed; cleanup terminates the job and
reaps the direct child. Job assignment/resumption failures kill and reap the
owned child. Windows host job restrictions may prevent creation and raise an
exception. Remaining descendants in the managed group/job are terminated when
the leader exits, including after a successful run.

The CLI maps Ctrl-C, and supported Unix termination/hangup signals, to cooperative
cancellation; pending child jobs are cleaned up and interrupted runs exit 130.
Abnormal host termination (for example SIGKILL, power loss, or an abort outside
normal cleanup) does not provide a portable cleanup guarantee.

### Explicit shell execution and permissions

`Process::shell(code, options = ...)` is a separately named operation requiring
both `process.execute` and `process.shell`. The shell capability is disabled by
default. Hosts must explicitly insert `process.shell` into
`PermissionConfig.allowed` to enable it. On Unix the interpreter is `/bin/sh -c`;
on Windows it is `cmd.exe /D /S /C`, with code supplied as a raw shell command
line rather than ordinary argv escaping. Code is intentionally shell-interpreted and
must not be formed from untrusted input without shell-specific handling.

On Windows, direct run/spawn reject `.bat` and `.cmd` program paths so batch files
cannot trigger implicit shell invocation. Use the explicit shell API for batch
syntax. Scripts may still explicitly select an interpreter executable as a direct
program; `process.execute` grants arbitrary external program execution. It is a
broad capability, not an OS sandbox or an enforcement mechanism for the File/Dir
root policy. Remove it when a host must not permit external programs.
`process.inspect` controls Process::exists. CurrentId needs no capability.

Errors include `CommandNotFoundError`, `ProcessError`, `PermissionError`,
`TimeoutError`, `OutputLimitError`, `EncodingError`, and `TypeError`. Errors retain
the native call span and script stack. Timeout/limit errors discard partial output
after cleanup instead of returning a successful result with truncated text.

## Environment

Environment state starts as an injectable snapshot of the host environment and
is mutable only within one runtime. Changes persist across calls, functions, and
execute calls, and are inherited by later children. They never call process-wide
`std::env::set_var` or `remove_var`. Per-child overrides do not mutate this snapshot.

| API | Behavior |
| --- | --- |
| `Env::get(name)` | String value or null if absent; an empty value is an empty string. |
| `Env::set(name, value): null` | Both arguments must be strings. Replaces/adds the runtime-local value. |
| `Env::has(name): bool` | Whether the name is present, including empty values. |
| `Env::remove(name): null` | Removes a name; an already absent name succeeds. |
| `Env::all(): map` | UTF-8 string keys/values, deterministically sorted by name. |

Names must be nonempty without `=` or NUL; values cannot contain NUL. Unix names
are case-sensitive. Windows ASCII environment names are matched case-insensitively.
Access to a non-UTF-8 environment value raises `EncodingError`; Env::all rejects
any unrepresentable entry. Children inherit native OS snapshot values without
lossy replacement. Read/has/all require `environment.read`; set/remove require
`environment.write`. Both capabilities are enabled by default.

## Script arguments and prompts

These forms expose exactly the same script argument list:

```sh
scr script.scr arg1 arg2
scr run script.scr arg1 arg2
```

The executable, `run`, and the script path are excluded. All tokens after the
script path belong to the script, including `--help`, `--version`, empty strings,
and `--`. `scr check` still accepts just one path. `scr run -- <path> ...` permits
paths starting with a hyphen or matching interpreter commands.

| API | Behavior |
| --- | --- |
| `Cli::args(): list` | All script arguments as UTF-8 strings, preserved verbatim. |
| `Cli::arg(index)` | Zero-based nonnegative int index; string or null if out of range. |
| `Cli::has(name): bool` | Whether an option is present before the first `--`. |
| `Cli::option(name)` | Last occurrence's value string, or null for absent/valueless options. |
| `Cli::readLine(maxLineBytes: int optional): string or null` | Reads a bounded UTF-8 stdin line without writing or flushing output. Default raw line limit is 1 MiB; EOF returns null. |
| `Cli::prompt(message)` | Writes and flushes a string prompt, reads one UTF-8 line, strips LF/CRLF, and returns it; EOF gives null. |

Option names may be queried as `name`, `--name`, or `-name`. Recognized forms are
`--name=value`, `--name value`, `-name=value`, and `-name value`. A following token
starting with a hyphen is treated as another option; use `--count=-1` for negative
values. `--name=` supplies an empty string. Repeated options use the last
occurrence, even when valueless. Short flags are not bundled: `-abc` is the name
`abc`. The first `--` stops option queries but stays in Cli::args. No quoting or
shell parsing is performed on script arguments. Non-UTF-8 arguments raise
`EncodingError` when exposed; their native encoding is preserved by CLI parsing.

`Cli::readLine` uses the runtime's buffered input (`Runtime::set_input_stream`)
and requires `cli.read`, enabled by default independently of `cli.prompt`.
`maxLineBytes` must be a positive int in platform range; invalid values raise
`RangeError` and wrong types/arity raise `TypeError` without consuming input.
The raw byte limit includes the LF/CRLF terminator. A line exactly at the limit
is accepted. LF and its preceding CR are removed; bare CR and a UTF-8 BOM remain.
Blank lines return `""`, final unterminated lines are returned, and EOF returns
`null` repeatedly. Unicode is decoded after assembling the bounded line.

Line reads check cancellation between chunks of at most 8 KiB and after reading.
The logical result budget includes a Value slot plus UTF-8 payload bytes, allowing
two terminator bytes while buffering. Size/budget/allocation failures raise
`ResourceLimitError`, invalid UTF-8 raises `EncodingError`, OS failures raise
`IoError`, and cancellation raises `Interrupted`. A failure inside the read
operation disables subsequent `Cli::readLine` and `Cli::prompt` calls for that
registration with `InputError`; no partial line is returned and no unbounded
attempt is made to drain the failed line. Permission/signature/range failures
leave input available. Replacing an embedding runtime's input reader does not
reset this failed state; use a fresh runtime/registration. A synchronous read
blocked waiting for input must return before cancellation can be observed.

`Cli::readLine` and `Cli::prompt` share the same buffered cursor. For example:

```scr
<?scr
while (($line = Cli::readLine(65536)) !== null) {
    echo $line . "\n";
}
```

Prompts use the runtime output writer and configurable buffered input
(`Runtime::set_input_stream`). They require `cli.prompt`, enabled by default.
The line limit is 64 KiB including its terminator; larger lines raise
`InputLimitError`. I/O and encoding failures are exceptions. Cancellation is
checked before and after input; an embedding reader blocked inside a read must
return before cancellation can be observed. Child stdin inheritance uses OS
stdin rather than this injected reader.


## System information

No System API launches an external command. Strings, signed 64-bit byte counts,
seconds, and finite percentage floats have the same meaning on Linux, macOS,
and Windows. `System::platform()` returns Rust's target OS identifier (`linux`,
`macos`, `windows`); `System::architecture()` returns the compiled target architecture
(e.g. `aarch64` or `x86_64`), which may differ from the physical host under emulation.
These two constant queries require no permission. Other queries require
`system.inspect`, enabled by default.

| API | Return and behavior |
| --- | --- |
| `System::hostname(): string` | Native hostname; unavailable/empty results raise `UnsupportedError`. |
| `System::cpuUsage(intervalMs = 200): float` | Blocking system-wide average CPU utilization from two counter observations, normalized across logical CPUs to 0–100 percent. |
| `System::memoryUsage(): MemoryUsage` | Physical **system RAM**, not this process's resident/virtual memory, swap, or container memory limit. |
| `System::diskUsage(path): DiskUsage` | Capacity of the filesystem containing an existing file/directory. Relative paths use runtime cwd. Requires `filesystem.read` and respects the host root policy. |
| `System::uptime(): int` | Whole seconds since OS boot, not runtime/process age. Zero/unavailable results raise `UnsupportedError`. |
| `System::homeDirectory(): string` | Native current user's home directory, using Rust's OS/environment lookup. |
| `System::tempDirectory(): string` | Rust's native temporary directory selection. Does not create or guarantee writability of the directory. |

`DiskUsage` is read-only, with `totalBytes: int`, `usedBytes: int`,
`availableBytes: int`, and `percentUsed: float`. Used bytes are total minus native
free bytes; available bytes are those available to the caller/nonprivileged user.
Reserved space/quotas can make used + available less than total.
Percent used is `100 * usedBytes / totalBytes`. A single native filesystem stats
query supplies these counters; an invalid/nonexistent path raises an I/O error.

`MemoryUsage` has the same read-only property names, but used bytes are explicitly
`totalBytes - availableBytes`. Available includes memory the OS considers
reclaimable, with OS-specific accounting of caches; it is not merely unallocated
RAM. The module does not claim to report process memory.

CPU calls create a fresh counter baseline, wait at least the requested interval,
then refresh the counters. The default is the larger of 200 ms and sysinfo's
platform minimum. An explicit interval must be at least that minimum and at most
60000 ms, otherwise `RangeError`; negative/noninteger durations are `TypeError`.
The sampling wait also requires `sleep` and checks cancellation every 10 ms.
Scheduling/refresh work can lengthen the actual interval. No cached first reading
or arbitrary instantaneous percentage is returned. A genuine idle reading is zero.

Host metrics use the existing sysinfo dependency. Unsupported OS backends, missing
CPU lists, missing hostname, and zero memory capacity raise `UnsupportedError`;
inconsistent counters raise `SystemError`, and byte counts beyond i64 raise
`RangeError`. Native filesystem failures remain explicit `IoError`. Some sysinfo
backends do not expose an error for every counter refresh failure; restricted
hosts can prevent observation, and the library cannot distinguish every such
backend failure from a genuine zero CPU reading. macOS sandbox restrictions and
restricted Linux `/proc` access may make capabilities unavailable. No missing
metric is synthesized. Home/temp lookups use the host environment, not the mutable
`Env` snapshot. Non-UTF-8 directory paths raise `EncodingError`.

```scr
<?scr
 echo "Host: " . System::hostname() . "\n";
 echo "Platform: " . System::platform() . "\n";
 $disk = System::diskUsage(System::tempDirectory());
 if ($disk->percentUsed > 85.0) echo "Disk capacity warning\n";
```

## Dates

| API | Behavior |
| --- | --- |
| `Date::now(): int` | Current wall-clock Unix **milliseconds**, UTC; may move backward when the system clock changes. |
| `Date::format(timestamp, format): string` | Formats signed Unix milliseconds in UTC using [Chrono strftime directives](https://docs.rs/chrono/latest/chrono/format/strftime/). No local-time or locale dependency. |
| `Date::parse(value): int` | Parses RFC 3339 with an explicit `Z` or numeric offset, returning UTC Unix milliseconds. Submillisecond precision is floored, including dates before 1970. |
| `Date::sleep(milliseconds): null` | Same nonnegative int duration, `sleep` permission, and cooperative cancellation as global `sleep`. Uses monotonic elapsed time. |

Invalid dates, missing timezones, unsupported leap seconds, and invalid format
directives raise `DateError`. Out-of-range formatting raises `RangeError` rather
than panicking. There is no heuristic parsing of ambiguous locale-specific dates.
For example, `Date::format(Date::now(), "%Y-%m-%dT%H:%M:%S%.3fZ")` emits UTC.

## Local logging

`Log::info(message)`, `Log::warn(message)`, `Log::error(message)`, and
`Log::debug(message)` accept exactly one string and return null. All four levels
are enabled and written consistently as lowercase `info`, `warn`, `error`, `debug`.
Each call synchronously writes one JSON line with `timestamp` (RFC 3339 UTC with
milliseconds), `level`, and `message`; JSON escaping prevents multiline messages
or control characters from injecting extra records. The default destination is
the runtime stderr writer, independent of script stdout.

`Log::setFile(path)` selects a local append-only file instead of stderr;
`Log::setFile(null)` restores stderr. The parent directory must exist. Selection
validates/open-creates the file before changing the destination; subsequent writes
recheck filesystem permissions and roots. No truncation or implicit directory
creation occurs. File paths resolve against runtime cwd at selection. Each log
call opens, appends, flushes, and closes the file; no background daemon, rotation,
network transport, remote collector, or environment-based network destination is
installed. Multiple runtimes/processes may interleave writes; atomicity across
writers is not promised. I/O failures are catchable `IoError` exceptions.

All log calls and destination changes require `log.write`, enabled by default.
File destinations additionally require `filesystem.write` and obey FilesystemPolicy
(including its documented concurrent-symlink limitations). Hosts can set a default
file and extra literal secrets through `StdlibOptions { log: LogConfig { file:
Some(path), redacted_values: vec![secret] }, ..Default::default() }`, importing
`scr_stdlib::log::LogConfig`. Relative host default paths use runtime cwd at write
time. Configuration and destination changes are local to the registered runtime.

Redaction happens before JSON serialization or I/O. Messages containing common
credential markers (password, secret, token, API key, authorization, credentials,
private keys), Bearer credentials, PEM headers, or URLs with userinfo are replaced
in full with `[REDACTED]`. Literal configured secrets and values of credential-named
variables in the runtime environment snapshot are replaced wherever they occur.
Empty secrets are ignored. Detection is deliberately conservative and may hide
benign messages containing these words. This cannot recognize every arbitrary
unmarked credential: callers should avoid passing secrets and hosts should
configure additional sensitive literal values. Logging never automatically dumps
the environment, process arguments, or exception context.


## JSON

`Json::encode(value): string` produces compact JSON; `Json::pretty(value): string`
uses two-space indentation. `Json::decode(string)` returns native Scripta values.
The JSON implementation uses serde_json without coercing native objects.

| Scripta | JSON |
| --- | --- |
| null | null |
| bool | boolean |
| int | exact signed 64-bit integer |
| finite float | JSON number (IEEE 754 f64 precision) |
| string | string, with Unicode preserved and required escaping |
| list | array |
| map with string keys | object |

Integer map keys raise `JsonError`: convert keys explicitly rather than silently
colliding with string keys. Native objects and callables also raise `JsonError`;
select their properties into a list/map first. Empty decoded objects remain maps,
and empty arrays remain lists. Object key order is deterministic (sorted), not a
preservation of source order. Duplicate keys in decoded JSON use the last value.

Integer JSON tokens outside i64 are rejected, including values beyond u64; they
are never silently rounded into floats. Tokens with a fractional part or exponent
become floats and must fit a finite f64. NaN, infinity, trailing content, invalid
escapes, malformed JSON, and nonfinite/out-of-range numbers raise `JsonError`.
Malformed JSON errors include the parser's one-based line/column location (columns
measure bytes). Numeric conversion errors identify the invalid number category.
Encoding is capped at 64 nesting levels and decoding at 128. Arguments and arity are checked through the native type system.

```scr
$data = Json::decode("{\"status\":\"up\",\"count\":3}");
echo Json::pretty($data) . "\n";
```

## HTTP

`Http::get(url, options = [])`, `Http::post(url, body, options = [])`, and
`Http::request(method, url, options = [])` make synchronous HTTP/HTTPS requests.
URLs must be absolute, have a host, and contain no embedded user credentials.
POST's body must be a string; JSON requests use `Json::encode(value)` explicitly
and supply `Content-Type: application/json`. Generic request bodies use the
`body` option. Supplying both POST's body argument and a body option is an error.
Methods are case-sensitive HTTP tokens; CONNECT tunnels are unsupported.

| Option | Default and behavior |
| --- | --- |
| `timeoutMs` | 30000; positive int within the platform clock range, covering the operation including redirects and body reads. |
| `headers` | Map of string header names to string values. Invalid names/newlines are rejected; Host, Content-Length, and Transfer-Encoding are managed by the client. |
| `maxBodyBytes` | 8388608 (8 MiB); nonnegative int, no higher than the host ceiling. Counts bytes before UTF-8 decoding. An oversized response is an error, never successful truncation. |
| `maxRedirects` | 5; nonnegative int, no higher than the host ceiling. Zero returns the original 3xx response without following it. |
| `body` | Empty string; raw request body for `Http::request`. |

Unknown options and invalid values raise `TypeError` before connecting. Headers
are optional; omit `headers` or use `headers: []` for an empty set. Each request returns a read-only
`HttpResponse` after the complete bounded body has been read:

| Member | Behavior |
| --- | --- |
| `status: int` | HTTP status code. |
| `headers: map` | Lowercase names mapped to **lists of strings**, preserving repeated header values. Header ordering is unspecified. |
| `body: string` | UTF-8 response text. Invalid UTF-8 raises `EncodingError`; binary bodies and charset conversion are not exposed yet. |
| `ok(): bool` | True for status 200–299, false otherwise. |
| `json()` | Decodes body using exactly `Json::decode` semantics, including line/column errors. Does not require a Content-Type header. |
| `text(): string` | Returns the retained body. Repeated calls do not consume it. |

HTTP 4xx/5xx responses are normal response values with `ok() == false`.
Transport failures return no response: `ConnectionError` covers connection/DNS/I/O
failures, `TlsError` covers certificate/handshake failures, `TimeoutError` covers
the deadline, `RedirectError` covers exhausted/unsupported redirects, and
`ResponseLimitError` covers excessive body size. Other protocol errors are
`HttpError`. Methods accept no arguments; bad arity is `TypeError`.

The blocking ureq client uses Rustls with WebPKI public certificate roots and
certificate/hostname validation enabled. There is no script option to bypass
validation. HTTPS requests cannot redirect down to HTTP. Client redirects follow
ureq semantics: 301/302/303 may change POST to GET; 307/308 with a non-replayable
request body raise `RedirectError`. Authorization is removed on redirected
requests by the client; custom headers can be forwarded, so callers should use
`maxRedirects: 0` when sending sensitive custom headers to untrusted endpoints.
No proxy is selected implicitly from the host environment; no cookie jar or
transparent response decompression is installed. Native DNS resolution may be
subject to OS resolver scheduling. Cancellation is checked before/after blocking
HTTP work; it cannot interrupt a pending socket operation before its timeout.

Requests require `http.request`, enabled by default. Hosts can remove that
capability or use `StdlibOptions { http: HttpConfig { timeout, max_response_bytes,
max_redirects }, ..Default::default() }` with `scr_stdlib::http::HttpConfig`.
Per-call timeout overrides must remain positive; body/redirect limits cannot
exceed host ceilings. HTTP responses never automatically enter logs.

```scr
<?scr
$response = Http::get("https://example.com/api/status");
if (!$response->ok()) throw "API request failed";
$data = $response->json();
echo $data["status"] . "\n";
```

See [local script loading](language-spec.md#local-script-loading) for reusable
helpers and configuration files. HTTP never acts as a remote module loader.


## Distribution and resource behavior

Every module above is statically linked into scr; there is no separate standard
library runtime to install. File/module/JSON/date/log/metrics examples can run
offline. Only an explicit Http call requires its endpoint, and only an explicit
Process call requires another executable. See [compatibility](compatibility.md)
for tested targets and [installation](install.md) for binary installation.

RuntimeOptions controls max_source_bytes (default 8 MiB), max_value_bytes,
max_scopes, instruction_limit, and bounded recursion. CLI equivalents are
SCR_MAX_SOURCE_BYTES, SCR_MAX_VALUE_BYTES, SCR_MAX_SCOPES, and SCR_MAX_STEPS.
Except source/value depth, new budgets are opt-in. A logical-value limit is not
an aggregate native-object/process heap ceiling. A tracing collector reclaims
unreachable captured environments during execution and at execution boundaries;
hosts can also request collection with `Runtime::collect_garbage()`. Host-retained
callables remain roots, and opaque native ownership cycles require host cleanup.
Freed storage need not immediately reduce RSS. See [safety details](performance.md)
for exact accounting and tests.

Diagnostics mask literal/comment excerpts and redact known credential values;
ContextOptions.redacted_values adds host-provided values. Script catch values and
explicit echo output retain their ordinary semantics. No implicit network logging
or background daemon is installed by library registration.
