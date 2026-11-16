# Standard library

Default CLI builds include [SQLite](sqlite.md): `Sqlite::open`, `Sqlite::execute`,
`Sqlite::query`, and `Sqlite::close`. No SQLite server, installed library, or
separate download is needed. Source builds can omit it with `--no-default-features`.
Embedders using `scr_stdlib::register` still register SQLite separately through
`scr_ext_sqlite`; its database handles and host policy remain in that crate.

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
| `type(value): string` | Returns `null`, `bool`, `int`, `float`, `string`, `bytes`, `list`, `map`, or `callable`. Native objects return their host-defined type name. |
| `len(value): int` | Counts Unicode scalar values in a string (not bytes or grapheme clusters), bytes in a byte value, or entries in a list/map. Other values are errors. |
| `sleep(milliseconds): null` | Requires a nonnegative int duration in milliseconds and the `sleep` capability, allowed by default. Zero returns immediately; long sleeps check cancellation in chunks of at most 10 ms of requested sleep. OS scheduling can delay observation. |

`int(1.5)` raises an error rather than truncating. `bool(2)`, `bool("")`, and
`bool("1")` raise errors rather than applying truthiness. Null is accepted only by
`str`, not by the numeric or bool conversions. Bytes, collections, native objects, and
callables are not accepted by any conversion. Non-finite floats are never valid.
Float string display need not retain a decimal point (`str(1.0)` is `"1"`), while
the original numeric types remain distinct for strict equality.

```scr
<?scripta
$usage = int("85");
echo "Usage: " . str($usage) . "%\n";
$ratio = float($usage) / 100.0;
if (bool("true")) echo str($ratio) . "\n";
```

Embedders opt in by calling `scr_stdlib::register(&mut runtime)?`. Registration
returns `NativeResult<()>`; duplicate names produce `RegistryError` and preserve
the existing registration. `echo` and `exit` remain language statements. Neither
is registered as a native function; exit returns a code without terminating the
embedding process. `File`, `Dir`, `Path`, `Process`, `Env`, `Cli`, `System`, `Date`, `Clock`, `Log`, `Json`, `Html`, `Password`, `Http`, `Url`, `Form`, `String`, `Math`, `List`, `Map`, `Bytes`, `Csv`, `Regex`, `Socket`, and `Stream` are
registered by default.

## Export index

This reference covers 203 core functions and four default CLI SQLite functions
(207 in total), plus 19 request-context Web/Session functions, native response/entry
properties, and local loading statements. No optional plugin,
external language runtime, shell, or network is required to use the local APIs.
Detailed signatures, return types, errors, permissions and limitations follow.

| Module | Public functions | Details |
| --- | --- | --- |
| Globals | `str`, `int`, `float`, `bool`, `type`, `len`, `sleep` | Conversion table above |
| String | `String::trim`, `String::split`, `String::join`, `String::contains`, `String::startsWith`, `String::endsWith`, `String::replace`, `String::substring`, `String::lower`, `String::upper`, `String::format`, `String::graphemeLength`, `String::graphemeSubstring` | [Strings](#strings) |
| Socket | `Socket::connect`, `Socket::listen`, `Socket::accept`, `Socket::bindDatagram`, `Socket::read`, `Socket::write`, `Socket::receive`, `Socket::sendTo`, `Socket::shutdown`, `Socket::close`, `Socket::setBlocking`, `Socket::setTimeout`, `Socket::poll`, `Socket::localAddress`, `Socket::peerAddress`, `Socket::setOption`, `Socket::getOption` | [Sockets](#low-level-sockets) |
| Stream | `Stream::open`, `Stream::request`, `Stream::read`, `Stream::write`, `Stream::flush`, `Stream::close`, `Stream::seek`, `Stream::tell`, `Stream::eof`, `Stream::copy`, `Stream::send`, `Stream::receive` | [General streams](#general-streams) |
| Output | `flush`, `ob_start`, `ob_flush`, `ob_end_flush`, `ob_clean`, `ob_end_clean`, `ob_get_contents`, `ob_get_clean`, `ob_get_flush`, `ob_get_length`, `ob_get_level` | [Output buffers](#output-buffers-and-flushing) |
| Password | `Password::hash`, `Password::verify`, `Password::needsRehash` | [Passwords](#passwords) |
| Html | `Html::escape` | [HTML escaping](#html-escaping) |
| Math | `Math::abs`, `Math::min`, `Math::max`, `Math::floor`, `Math::ceil`, `Math::round` | [Math](#math) |
| List | `List::remove`, `List::slice`, `List::concat`, `List::contains`, `List::sort`, end operations, generation, search, aggregation, comparisons, `List::column` | [Collections](#collections) |
| Map | `Map::create`, `Map::has`, `Map::get`, `Map::keys`, `Map::values`, `Map::remove`, merge/replace, combine/flip, search/reverse, comparisons, `Map::column` | [Collections](#collections) |
| File | `File::exists`, `File::read`, `File::readBytes`, `File::writeBytes`, `File::appendBytes`, `File::write`, `File::writeAtomic`, `File::append`, `File::copy`, `File::move`, `File::delete`, `File::size`, `File::modified`, `File::extension`, `File::openLines`, `File::readLine`, `File::close` | [Filesystem](#filesystem-and-paths) |
| Dir | `Dir::exists`, `Dir::create`, `Dir::createAll`, `Dir::files`, `Dir::directories`, `Dir::entries`, `Dir::remove`, `Dir::current`, `Dir::change` | [Filesystem](#filesystem-and-paths) |
| Path | `Path::join`, `Path::filename`, `Path::extension`, `Path::parent`, `Path::normalize`, `Path::absolute`, `Path::canonical` | [Paths](#filesystem-and-paths) |
| Process | `Process::run`, `Process::spawn`, `Process::shell`, `Process::wait`, `Process::poll`, `Process::kill`, `Process::exists`, `Process::currentId` | [Processes](#processes) |
| Env | `Env::get`, `Env::set`, `Env::has`, `Env::remove`, `Env::all` | [Environment](#environment) |
| Cli | `Cli::args`, `Cli::arg`, `Cli::has`, `Cli::option`, `Cli::isTerminal`, `Cli::prompt`, `Cli::readLine` | [CLI](#script-arguments-and-prompts) |
| System | `System::platform`, `System::architecture`, `System::hostname`, `System::cpuUsage`, `System::memoryUsage`, `System::diskUsage`, `System::uptime`, `System::homeDirectory`, `System::tempDirectory` | [System](#system-information) |
| Date | `Date::now`, `Date::format`, `Date::parse`, `Date::sleep` | [Dates](#dates) |
| Clock | `Clock::monotonicMs` | [Monotonic timing](#monotonic-timing) |
| Log | `Log::info`, `Log::warn`, `Log::error`, `Log::debug`, `Log::setFile` | [Logging](#local-logging) |
| Json | `Json::encode`, `Json::decode`, `Json::pretty` | [JSON](#json) |
| Bytes | `Bytes::encodeUtf8`, `Bytes::decodeUtf8`, `Bytes::slice` | [Binary values](#binary-values) |
| Csv | `Csv::parse`, `Csv::encode`, `Csv::open`, `Csv::readRow`, `Csv::close` | [CSV](#csv) |
| Regex | `Regex::isMatch`, `Regex::findAll`, `Regex::replace` | [Regex](#regex) |
| Web (request context only) | `Web::request`, `Web::status`, `Web::header`, `Web::addHeader`, `Web::redirect`, `Web::write`, `Web::download`, `Web::cookie`, `Web::deleteCookie` | [Web requests](#web-requests) |
| Session (request context only) | `Session::start`, `Session::id`, `Session::get`, `Session::has`, `Session::set`, `Session::remove`, `Session::all`, `Session::regenerate`, `Session::destroy`, `Session::close` | [Sessions](#sessions) |
| Http | `Http::get`, `Http::post`, `Http::request`, `Http::download` | [HTTP](#http) |
| Url | `Url::encodeComponent`, `Url::decodeComponent`, `Url::buildQuery`, `Url::parseQuery` | [URL and form encoding](#url-and-form-encoding) |
| Form | `Form::encode`, `Form::decode` | [URL and form encoding](#url-and-form-encoding) |
| Sqlite (CLI default) | `Sqlite::open`, `Sqlite::execute`, `Sqlite::query`, `Sqlite::close` | [SQLite](sqlite.md) |

`DirEntry`, `ProcessResult`, `ProcessHandle`, `MemoryUsage`, `DiskUsage`,
`HttpResponse`, `CsvReader`, `FileStream`, `ByteStream`, `DatagramSocket`, and `SocketListener` are read-only native objects whose properties
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
and [the functions playground](../playground/functions.scripta). Callback-based
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

`String::graphemeLength(text: string): int` counts extended grapheme clusters.
`String::graphemeSubstring(text: string, start: int, length: int optional): string`
uses those clusters as its position unit. Bounds and omitted length behave like
`String::substring`; invalid bounds raise `RangeError`. These helpers preserve
skin-tone modifiers, flag pairs, combining accents, and joined emoji sequences.
For example, `len("👍🏽")` is 2, while `String::graphemeLength("👍🏽")` is 1.
Segmentation uses Unicode 16.0 tables from `unicode-segmentation` 1.12.0;
clusters do not imply a particular terminal width or font rendering. No
normalization is applied. Existing scalar-based operations retain their semantics.

`String::format` accepts exactly one value per `{}` placeholder. Values may be
null, bool, int, finite float, or string, with the same display as `str` (so 1.0
prints as `1`). Replacement strings are inserted literally and never reparsed.
For example, `String::format("{{{}}}: {}", ["café", 2])` returns `"{café}: 2"`.
Empty templates with empty value lists return `""`. Unmatched braces, unsupported
placeholder syntax, and missing or extra values raise `FormatError`; unsupported
value types raise `TypeError`. There are no width, precision, positional, or named
placeholders. Unicode is preserved without normalization. Output is preflighted
before allocating the result and uses the byte budget described below.

Matching in the literal `String` search helpers is case-sensitive, without Unicode
normalization. Use the separate `Regex` module for patterns. For example, precomposed `é` and `e` followed by a combining acute
accent are different searches. Substring positions use the same Unicode scalar
unit as `len`, not bytes or grapheme clusters; a combining mark can be selected
separately. Trimming and case conversion use Rust's Unicode tables, independently
of the operating system locale.

```scr
<?scripta
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

Try [the strings playground](../playground/strings.scripta) for a complete status
record normalization example.

## Binary values

Strings remain valid UTF-8 text. `bytes` is a separate immutable value holding
arbitrary bytes, including NUL and invalid UTF-8, with content equality and cheap
shared clones. It is accepted by function parameter/return annotations as `bytes`.
It has no implicit string conversion, concatenation, indexing, or JSON encoding.
Use explicit conversion or byte slicing:

| Function | Behavior |
| --- | --- |
| `Bytes::encodeUtf8(text: string): bytes` | Encodes the string as UTF-8 bytes. |
| `Bytes::decodeUtf8(data: bytes): string` | Strict decoding; invalid UTF-8 raises `EncodingError`. |
| `Bytes::slice(data: bytes, start: int, length: int optional): bytes` | Nonnegative byte positions; omitted length returns the suffix. Start at the end is allowed; invalid bounds raise `RangeError`. A slice can split a UTF-8 sequence. |
| `len(data: bytes): int` | Number of bytes, independent of text encoding. |

Byte payloads, including those nested in collections, count toward the runtime
logical value byte limit. Shared values are charged by logical size, not pointer
size. File and HTTP operations return byte values without decoding or replacing
any data.

## Regex

These functions use Rust's `regex` crate with Unicode matching enabled. Arguments
are strings. Patterns have no surrounding delimiters; use inline flags such as
`(?i)` for case-insensitive matching and `(?m)` for multiline anchors. Escape a
backslash in a double-quoted Scripta string, e.g. `"\\d+"`.

| Function | Behavior |
| --- | --- |
| `Regex::isMatch(text, pattern): bool` | Whether the pattern matches anywhere in the text. Use anchors for a full-text match. |
| `Regex::findAll(text, pattern): list` | Full matched strings in left-to-right, nonoverlapping order; empty list for no match. |
| `Regex::replace(text, pattern, replacement): string` | Replaces all nonoverlapping matches with the literal replacement. `$1`, backslashes, and capture names have no special replacement meaning. |

Empty patterns are valid and match at Unicode scalar boundaries, including both
ends. Matching does not normalize text or treat grapheme clusters as a unit.
Lookaround and pattern backreferences are unsupported. Invalid or oversized
patterns raise `RegexError`; patterns are limited to 65,536 UTF-8 bytes, compiled
regex size to 1 MiB, and DFA cache size to 1 MiB. Captures, compiled script objects,
callbacks, and regex splitting are not exposed. Output growth is checked against
runtime value limits before each append; cancellation is checked between matches.
A single native match/compile runs synchronously.

## CSV

CSV uses the Rust `csv` crate behind a bounded record scanner. Parsing consumes
complete records across physical lines, handling quoted delimiters, doubled
quotes, embedded newlines, CRLF/LF/CR terminators, and a final unterminated line.
Fields are UTF-8 strings without numeric coercion, trimming, or normalization.
Backslashes are literal; quotes are escaped by doubling them. An initial UTF-8 BOM is
removed; BOM characters elsewhere are retained as field content.

| Function | Behavior |
| --- | --- |
| `Csv::parse(text: string, options optional): list` | Returns a list of rows, each a list of strings. Empty input returns an empty list. |
| `Csv::encode(rows: list, options optional): string` | Writes rows of strings with necessary quoting and CRLF record terminators. Empty rows input returns an empty string. |
| `Csv::open(path: string, options optional): CsvReader` | Opens a regular file with the same filesystem policy as `File::read`. |
| `Csv::readRow(reader: CsvReader): list or null` | Reads a complete record; null at EOF. Reading an explicitly closed reader raises `StreamError`. |
| `Csv::close(reader: CsvReader): null` | Idempotently closes the reader and all aliases; `reader->closed` reports state. |

Options are a map or empty list. `delimiter` defaults to comma and must be one
ASCII byte other than NUL, quote, CR, or LF. `maxRecordBytes` defaults to 1 MiB,
is a positive int up to 16 MiB, and counts raw record bytes including quoting and
terminators. Unknown options and invalid types raise `TypeError`. Record and
execution value limit violations raise `ResourceLimitError`.

Rows must have consistent, nonzero field counts. Blank input records produce one
empty field. Malformed quotes, invalid UTF-8, and inconsistent widths raise
`CsvError` with record context. No header interpretation is performed: the first
row is ordinary data, and duplicate header text is retained. Applications can
validate and map headers explicitly. CSV encoding preserves field contents; it
does not sanitize spreadsheet formulas.

Readers retain an open descriptor, close on read errors, and release their
resources when dropped. They cannot be used by another standard-library
registration. Use `try`/`finally` to close them promptly. `Csv::parse` bounds the
entire output value; streaming bounds one raw record and returned row at a time.

```scr
<?scripta
$reader = Csv::open("records.csv");
try {
    while (($row = Csv::readRow($reader)) !== null) {
        echo Json::encode($row) . "\n";
    }
} finally {
    Csv::close($reader);
}
```

## Passwords

| Function | Behavior |
| --- | --- |
| `Password::hash(password: string): string` | Hash using Argon2id v19, a fresh 16-byte OS-random salt, and a 32-byte digest. Returns the complete PHC-format string. |
| `Password::verify(password: string, hash: string): bool` | Verify an Argon2id v19 hash using its bounded encoded parameters. Wrong passwords and malformed/unsupported hashes return false. |
| `Password::needsRehash(hash: string): bool` | True for malformed/unsupported hashes, or supported hashes whose costs, salt length, or digest length differ from the current generation policy. No password hashing is performed. |

Inputs are strictly strings. Passwords use their exact UTF-8 bytes, including
embedded NUL; there is no normalization, trimming, coercion, or truncation. Empty
passwords are accepted by the primitive; application password policy is separate.
Store the entire returned string, which contains the algorithm, version, costs,
salt, and digest. Salts are automatic and cannot be supplied by scripts.

The default generation policy is 19456 KiB (19 MiB) of working memory, two passes,
and one lane. It follows the [OWASP Argon2id minimum configuration](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id).
The Rust host can configure `StdlibOptions.password: password::PasswordConfig`;
CLI and web hosts use its defaults. Scripts cannot override or raise the policy.

| Host field | Default | Meaning / absolute host bounds |
| --- | --- | --- |
| `memory_kib` | `19456` | Generation memory, satisfying Argon2 constraints and host ceilings. |
| `iterations` | `2` | Generation passes, within host ceilings. |
| `parallelism` | `1` | Generation lanes; not a multiplier of total memory or a promise of OS threads. |
| `max_memory_kib` | `65536` | Per-call verification memory ceiling; 8–262144 KiB. |
| `max_iterations` | `10` | Per-call pass ceiling; 1–32. |
| `max_parallelism` | `4` | Per-call lane ceiling; 1–16. |
| `max_work_kib` | `262144` | Ceiling on memory KiB times passes; 1–8388608 KiB-passes. |
| `max_password_bytes` | `1024` | UTF-8 input byte ceiling; configurable from 1 to 1048576. |
| `max_hash_bytes` | `512` | Encoded hash byte ceiling; at most 4096 and large enough for configured generation output. |

All ceilings apply to generation too. Invalid host settings fail registration
with `PasswordError`. Generation parameters must also satisfy Argon2's minimum
memory/lane/pass constraints. Low-cost settings used in tests are not production
recommendations. Parameter changes are trusted host decisions, not request data.

Verification accepts complete Argon2id v19 PHC hashes with exactly `m`, `t`, and
`p` parameters, a decoded salt of 8–48 bytes, and a digest of 16–64 bytes.
Parameter order does not affect the policy comparison. Bcrypt, Argon2i/d, older
versions, pepper/key-id/associated-data extensions, and incomplete hashes are
unsupported. PHP Argon2id hashes are interoperable within the host ceilings;
this does not imply compatibility with PHP's default bcrypt algorithm.

Password/hash byte-ceiling violations and parsed costs above policy raise
`PasswordError` rather than returning false. Limits are checked before working
memory allocation or expensive computation. A parsed cost violation takes
precedence over later salt/parameter validation. Other backend/OS-random failures
also raise generic `PasswordError`, without including input values. Signature
errors remain `TypeError`. Failed assignments preserve the prior destination.

`needsRehash` compares against the exact generation policy, not a strength score;
a different, higher-cost hash can also return true. Only rehash after successful
verification, with an intentionally chosen host policy:

```scr
<?scripta
if (Password::verify($password, $storedHash)) {
    if (Password::needsRehash($storedHash)) {
        $replacementHash = Password::hash($password);
        // Save the complete replacement hash using a parameterized database query.
    }
}
```

The final digest comparison uses RustCrypto's constant-time `Output` comparison.
The entire API call is not constant-time across different algorithms, malformed
inputs, or costs, and it does not implement a complete authentication workflow.
See the [runnable example](../examples/password-hashing.scripta).

Hash output and bool results respect logical value budgets; generation preflights
the exact output size before obtaining randomness or allocating working memory.
Working memory uses a separately bounded, fallibly reserved allocation and is
zeroized on normal completion/unwinding. This is not a process heap cap or a
promise to erase all copies of script password strings. No automation capability
is required; these functions perform computation and use OS entropy internally.
No public secure-random or token-comparison API is added here.

Cancellation is checked before work, during buffer initialization, and after the
backend returns. The Argon2 computation itself cannot be cooperatively interrupted.
Web request deadlines can terminate its isolated worker; CLI/embedding callers may
wait for the bounded computation to finish before interruption is reported.
Concurrent calls/workers multiply the per-call memory allowance: four default
web workers hashing concurrently use about 76 MiB of Argon2 workspace, while four
verifications at the default ceiling can use up to 256 MiB, plus other runtime
memory. Choose worker counts and host policies for the deployment's memory budget.
Instruction/checkpoint budgets do not count Argon2's internal operations.

## HTML escaping

`Html::escape(text: string): string` escapes plain UTF-8 text for HTML text nodes
and ordinary quoted attribute values, such as `title`. It takes exactly one
string; other types require explicit conversion and invalid signatures raise
`TypeError`. It performs no I/O and requires no capabilities.

| Input character | Output |
| --- | --- |
| `&` | `&amp;` |
| `<` | `&lt;` |
| `>` | `&gt;` |
| `"` | `&quot;` |
| `'` | `&#039;` |

All other Unicode scalars, whitespace, and control characters are unchanged; the
function does not normalize text or validate arbitrary HTML. Empty input returns
an empty string. Existing entities are escaped again: `&amp;` becomes `&amp;amp;`.
Generated entities are not processed a second time within the same call. Keep
plain text in storage and escape once when inserting it into HTML.

```scr
<?scripta
$name = "Ada <Admin> & \"friends\"";
$escaped = Html::escape($name);
echo "<p title=\"" . $escaped . "\">Hello, " . $escaped . "!</p>\n";
```

Use quotation marks around dynamic attribute values. This function does not
encode JavaScript, CSS, URL components, or SQL; event-handler/style attributes
and URL-valued attributes need their own validation/encoding. It does not sanitize
user-supplied HTML markup. See [URL encoding](#url-and-form-encoding) for URL
components and the [runnable example](../examples/html-escaping.scripta).

The implementation preflights the exact UTF-8 output bytes, including the logical
Value slot, before reserving the result. Overflow/allocation failures and byte
ceilings raise `ResourceLimitError`. Both sizing and output passes check
cancellation, including empty input and completion; failures preserve the previous
assignment destination. No HTML is escaped automatically by `echo`.

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

Try [the reporting playground](../playground/reporting.scripta):
`cargo run -p scr-cli --locked -- run playground/reporting.scripta`.

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
| `List::push(items: list, value: any): list` | Appends one value to a copy. Use `concat` to append another list's elements. |
| `List::unshift(items: list, value: any): list` | Prepends one value to a copy. |
| `List::pop(items: list): map` | Removes the last value and returns `["items" => remainingList, "value" => removedValue]`. Empty input raises `RangeError`; a removed null remains null. |
| `List::shift(items: list): map` | Like `pop`, but removes the first value. |
| `List::splice(items: list, start: int, length: int, replacement: list optional): map` | Returns `["items" => updatedList, "removed" => removedList]`. Replacement defaults to `[]`. Start and length must be nonnegative, start cannot exceed the list length, and length cannot exceed the remainder; violations raise `RangeError`. Zero length inserts without removing. Start at the end permits appending. Explicit null is not an omitted replacement. |
| `List::reverse(items: list): list` | Reverses element order. Empty input returns `[]`. |
| `List::chunk(items: list, size: int): list` | Returns contiguous sublists of at most size elements, with a possibly shorter final sublist. Size must be positive, otherwise `RangeError`. Empty input returns `[]`. |
| `List::fill(count: int, value: any): list` | Generates count independent copies of value. Count must be nonnegative, otherwise `RangeError`; zero returns `[]`. |
| `List::search(items: list, value: any): int or null` | Returns the first strictly equal value's index, or null when absent. Index zero is a successful match. |
| `List::unique(items: list): list` | Keeps the first occurrence of each value using strict recursive equality. Preserves order. |
| `List::sum(items: list): int or float` | Adds homogeneous ints or finite floats in input order. Empty input returns integer `0`. Mixed types and nonnumeric/nonfinite inputs raise `TypeError`; integer overflow and nonfinite intermediate float results raise `ArithmeticError`. |
| `List::product(items: list): int or float` | Multiplies with the same numeric rules as `sum`. Empty input returns integer `1`. |
| `List::diff(left: list, right: list): list` | Keeps left values absent from right, using strict recursive equality. Preserves left order and duplicates. |
| `List::intersect(left: list, right: list): list` | Keeps left values present in right, using strict recursive equality. Preserves left order and duplicates; right duplicates do not multiply results. |
| `List::column(rows: list, field: int or string): list` | Extracts field from each map row in order. Non-map rows raise `TypeError`, missing fields raise `RangeError`, and stored null is retained. Empty rows return `[]`. |
| `Map::merge(left: map, right: map): map` | Right values replace matching typed keys without moving their positions. New keys append in right order. Numeric keys are preserved. |
| `Map::replace(left: map, right: map): map` | Like merge, but ignores right keys absent from left. |
| `Map::combine(keys: list, values: list): map` | Builds a map from equally sized lists, otherwise `RangeError`. Keys must be ints or strings, otherwise `TypeError`. Later duplicate keys overwrite values while retaining their first insertion position. Empty lists produce an empty map. |
| `Map::flip(items: map): map` | Exchanges values and keys. Input values must be ints or strings, otherwise `TypeError`. Duplicate generated keys use the same overwrite and insertion rule as combine. |
| `Map::search(items: map, value: any): int or string or null` | Returns the first strictly equal value's typed key in insertion order, or null when absent. |
| `Map::reverse(items: map): map` | Reverses entry order while preserving typed keys. |
| `Map::diff(left: map, right: map): map` | Keeps left entries whose values are absent from right, using strict recursive equality. Preserves left keys and order. |
| `Map::intersect(left: map, right: map): map` | Keeps left entries whose values occur in right, using strict recursive equality. Preserves left keys and order. |
| `Map::diffKeys(left: map, right: map): map` | Keeps left entries whose exact typed keys are absent from right. Preserves left order. |
| `Map::intersectKeys(left: map, right: map): map` | Keeps left entries whose exact typed keys occur in right. Preserves left order. |
| `Map::column(rows: list, field: int or string, indexField: int or string): map` | Extracts field values keyed by each row's indexField value. Rows must be maps and both fields must exist, with the same errors as List column. Index values must be ints or strings. Duplicate indexes overwrite values without moving their first insertion position. Empty rows return an empty map. |

Arguments are strictly typed; invalid types and argument counts raise catchable
`TypeError`. Helpers never coerce keys, create holes, or create missing intermediate
collections. `Map::combine` and `Map::column` explicitly construct maps from
lists. Sorting uses the language's numeric and UTF-8 string ordering, with no locale collation or numeric coercion. Stable
float sorting preserves the order of equal values, including positive and
negative zero.

```scr
<?scripta
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
per value, UTF-8 string bytes, byte buffers, reported native-object payloads,
and MapKey slots plus string-key bytes. Growing map and column output plans
check sizes before reserving additional entries. When duplicate keys may still
overwrite values, the early check counts keys and Value slots; the complete
winning values are checked before copying, so overwritten payloads do not cause
premature failures. Detected
size overflows, byte-budget violations, and fallible result reservations raise
`ResourceLimitError`; input arguments and returned booleans also remain subject
to the runtime's ordinary value checks. These limits do not bound process RSS
or temporary key indexes and sorting scratch space. Cancellation is checked at native-call boundaries
and during result traversal/copying; a sort can delay cancellation while it is
ordering the result. Failed helpers leave assignment destinations unchanged.

`Map::merge`, `replace`, `diffKeys`, and `intersectKeys` use temporary hash
indexes of borrowed typed keys. `combine`, `flip`, and indexed `column` use
indexes of the first insertion positions. Their key lookups take expected
linear total work in the number of entries, plus string hashing and result
traversal/copying. Ordered vectors determine all result order, including after
overwrites. Index reservations are fallible and index construction checks
cancellation between entries. Value-based `diff` and `intersect` retain strict
recursive comparisons and can take quadratic time.

`Map::get` and `has` provide safe lookups without exceptions. Direct missing-key
indexing and invalid list positions raise catchable core `IndexError`; invalid
core operand/index types raise `TypeError`, and arithmetic overflow or division
by zero raises `ArithmeticError`. Core runtime safety limits and cancellation
remain outside catches. Existing helper `RangeError` and `ResourceLimitError`
exceptions are unchanged. See the [exception contract](language-spec.md#statement-grammar)
and [errors playground](../playground/errors.scripta). Callback transforms and recursive merge variants
are deferred. Try the [collections playground](../playground/collections.scripta)
and [log analyzer](../playground/log-analyzer.scripta) for complete examples.


### PHP array function mapping

These helpers return new values. PHP functions that mutate arrays need an
explicit assignment in Scripta; pop and shift also return the removed value.
Comparisons are always strict, without PHP's default coercion.

| PHP function | Scripta equivalent | Difference |
| --- | --- | --- |
| `array_push`, `array_unshift` | `List::push`, `List::unshift` | One value per call; returns a new list rather than its length. |
| `array_pop`, `array_shift` | `List::pop`, `List::shift` | Returns items and value; empty input raises `RangeError`. |
| `array_splice` | `List::splice` | Returns items and removed; requires nonnegative, in-bounds start and length. |
| `array_reverse` | `List::reverse`, `Map::reverse` | Map keys are always preserved. |
| `array_chunk` | `List::chunk` | List input only. |
| `array_fill` | `List::fill(count, value)` | Generates a list starting at index zero; no start-index argument. |
| `array_search` | `List::search`, `Map::search` | Always strict; missing returns null. |
| `array_unique` | `List::unique` | Strict recursive equality and contiguous output indexes. |
| `array_sum`, `array_product` | `List::sum`, `List::product` | Homogeneous numeric types, checked arithmetic, no coercion. |
| `array_merge` | `List::concat`, `Map::merge` | Map integer keys are preserved and overwritten like string keys. |
| `array_replace` | `Map::merge` | `Map::replace` instead changes existing keys only. |
| `array_combine`, `array_flip` | `Map::combine`, `Map::flip` | Integer and string keys remain distinct. |
| `array_diff`, `array_intersect` | List or Map `diff`, `intersect` | Always strict; map results retain left keys. |
| `array_diff_key`, `array_intersect_key` | `Map::diffKeys`, `Map::intersectKeys` | Typed key comparison. |
| `array_column` | `List::column`, `Map::column` | Requires map rows and existing fields; indexed output requires valid typed keys. |

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
| `permissions` | Automation capabilities are allowed by default: sleep, filesystem read/write, environment read/write, process execute/inspect, system.inspect, log.write, http.request, network.connect, network.datagram, network.bind, network.listen, cli.read, and cli.prompt. `process.shell` remains opt-in. |
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
| `File::readBytes(path: string): bytes` | Reads a complete regular file without text decoding, subject to the execution value limit. |
| `File::writeBytes(path: string, contents: bytes): null` | Creates or truncates a regular file and writes the exact bytes. |
| `File::appendBytes(path: string, contents: bytes): null` | Creates a missing file or appends exact bytes. |
| `File::write(path, contents): null` | Creates or truncates a regular file, replacing its contents. Parent directories must exist. |
| `File::writeAtomic(path: string, contents: string): null` | Stages a complete UTF-8 file beside the destination, then atomically creates or replaces it. Parent directory must exist; final symlinks, nonregular files, and read-only destinations are rejected. |
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

### Atomic file replacement

`File::writeAtomic(path, contents)` requires `filesystem.write` and the registered
filesystem write/root policy, including access to the destination's parent.
It creates an exclusively named `.scripta-write-*` temporary file in that parent,
writes in 64 KiB chunks with cancellation checks, applies supported permissions,
synchronizes the staged file, closes its data handle, and publishes it with a
same-filesystem rename/replacement. It never deletes the destination first or
falls back to copying over it. Both arguments must be strings; normal path
validation and runtime value limits apply. It returns null on success.

At the destination path, concurrent readers opening the file see complete old or
new contents on filesystems that support atomic replacement. Readers holding an
old Unix descriptor keep reading the old file. Concurrent writers use
last-successful-replacement behavior; there is no compare-and-swap or multi-file
transaction. Windows handles opened without delete sharing may block replacement;
transient Windows access/sharing/lock errors are retried within a 250 ms window,
with at most 5 ms sleeps and cancellation/destination-policy checks between
attempts. Persistent errors raise an exception and leave the old file in place.
This retry window is not a hard deadline for blocking OS calls and does not add
a delete/copy fallback.

Existing regular files must allow OS write access and must not be read-only.
On Unix, replacement preserves the currently observed ordinary rwx permission
bits; setuid, setgid, and sticky bits are stripped. New Unix files use private
`0600` permissions, subject to umask. On Windows, new files inherit directory
ACLs; custom target ACLs are not copied. Replacement uses a new file owned under
the writer's normal creation rules: ownership/group, timestamps, extended
attributes, hard-link identity, and other metadata are not preserved. Existing
hard links continue to name the old file. For public web reports, grant reader
access once after initial creation; Unix mode bits then survive replacements.

Signature, permission, path, read-only, write, synchronization, and replacement
failures use the existing TypeError/PathError/PermissionError/filesystem error
categories. Cancellation is checked before commit; no cancellation check or other
fallible operation follows a successful commit. Failures before publication leave
the destination unchanged by this operation. Temporary files are explicitly
removed on ordinary failure/cancellation, with cleanup failures appended to the
original error. Abrupt termination may leave a staging file behind.

File synchronization reduces crash exposure, but the parent directory is not
synchronized: this API does not guarantee power-loss durability. Filesystem root
and symlink checks retain their existing concurrent-path-race limitations; this
is not an adversarial filesystem sandbox. Atomicity ultimately depends on the
underlying filesystem, especially for network or unusual mounts. Each platform
needs native validation before support is claimed.

Try [atomic-report.scripta](../examples/atomic-report.scripta): preview with
`scripta examples/atomic-report.scripta report.json`, then add `--apply` to publish it.

| Directory method | Behavior |
| --- | --- |
| `Dir::exists(path): bool` | True for a directory; false for missing paths, files, and final symlinks. Other errors propagate. |
| `Dir::create(path): null` | Creates one new directory; its parent must exist. Any existing name raises an error. |
| `Dir::createAll(path): null` | Creates missing parents; an existing directory succeeds. Files and final symlinks are rejected. |
| `Dir::files(path): list` | Shallow list of regular-file `DirEntry` snapshots, sorted by UTF-8 name in case-sensitive lexical order. |
| `Dir::directories(path): list` | Shallow list of directory snapshots in the same order. |
| `Dir::entries(path): list` | Shallow, sorted list of maps with `name`, absolute `path`, `isDirectory`, `isFile`, and `isSymlink`. Includes dangling links and special files without resolving targets. Uses the execution value-byte limit before growing the result. |
| `Dir::remove(path): null` | Removes only an empty directory. Rejects root, current directory, and its ancestors. There is no recursive-delete argument or API. |
| `Dir::current(): string` | Canonical absolute runtime working directory. |
| `Dir::change(path): null` | Requires an existing directory and saves its canonical absolute path in this runtime. |

`Dir::files` and `Dir::directories` exclude symlinks and special files; no listing
recurses. `Dir::entries` explicitly exposes links (including Windows junctions)
as link snapshots; type flags describe the link itself, not its target.
Existing file/directory operations continue to reject final symlinks except
existence checks (false) and lexical extension extraction. Intermediate symlink directories use OS resolution; the
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

`Path::canonical` is the explicit filesystem-resolving exception to the lexical
Path helpers: it requires `filesystem.read`, follows links to an existing target,
and returns its canonical absolute path without changing the runtime directory.
It enforces host root policies on the destination. Dangling links raise
`FileNotFoundError`; resolution/permission/encoding errors propagate. This does
not change existing read/write symlink rules or their concurrent-path-race limits.
Try [directory-links.scripta](../examples/directory-links.scripta) with a fixture directory.

| Path method | Behavior |
| --- | --- |
| `Path::canonical(path): string` | Resolve an existing filesystem path, following links under read/root policies. |
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
<?scripta
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
or asynchronous I/O. The [streaming playground](../playground/streaming.scripta)
counts sample log records with bounded per-line input and constant-size counters:
`cargo run -p scr-cli --locked -- run playground/streaming.scripta playground/sample-events.txt`.

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
<?scripta
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
<?scripta
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
scripta script.scripta arg1 arg2
scripta run script.scripta arg1 arg2
```

The executable, `run`, and the script path are excluded. All tokens after the
script path belong to the script, including `--help`, `--version`, empty strings,
and `--`. `scripta check` still accepts just one path. `scripta run -- <path> ...` permits
paths starting with a hyphen or matching interpreter commands.

| API | Behavior |
| --- | --- |
| `Cli::args(): list` | All script arguments as UTF-8 strings, preserved verbatim. |
| `Cli::arg(index)` | Zero-based nonnegative int index; string or null if out of range. |
| `Cli::has(name): bool` | Whether an option is present before the first `--`. |
| `Cli::option(name)` | Last occurrence's value string, or null for absent/valueless options. |
| `Cli::isTerminal(stream: string): bool` | Whether the process's `"stdin"`, `"stdout"`, or `"stderr"` handle connects to a terminal. |
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

`Cli::isTerminal` accepts exactly `"stdin"`, `"stdout"`, and `"stderr"`
(case-sensitive). Other names, wrong argument types, and wrong arity raise
`TypeError`. It returns false for pipes, redirected files, null devices, or when
the OS cannot determine terminal status. It neither reads nor writes a stream
and does not require `cli.read` or `cli.prompt` permission. In embedded runtimes,
it inspects the host process's standard handles, not injected input, output, or
error writers.

Terminal detection does not guarantee ANSI color or cursor-control support and
does not interpret `NO_COLOR` or CLI options. Scripts decide those policies:

```scr
<?scripta
$useColor = Cli::isTerminal("stdout")
    && !Env::has("NO_COLOR")
    && Cli::option("color") !== "never";

if (Cli::isTerminal("stdin") && Cli::isTerminal("stdout")) {
    $name = Cli::prompt("Your name: ");
}
```

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
<?scripta
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
Path existence is checked before the capacity query, including on Windows where
the native volume lookup alone can succeed for a nonexistent child path.

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
<?scripta
 echo "Host: " . System::hostname() . "\n";
 echo "Platform: " . System::platform() . "\n";
 $disk = System::diskUsage(System::tempDirectory());
 if ($disk->percentUsed > 85.0) echo "Disk capacity warning\n";
```

## Monotonic timing

`Clock::monotonicMs(): float` returns nonnegative, nondecreasing milliseconds from
an arbitrary process-local origin, initialized by the first clock read. It uses
Rust's monotonic `Instant`, independently of the adjustable date/time clock.
The origin is shared across runtimes in the same process. Use differences between
readings to measure elapsed work; values are not Unix timestamps and must not be
compared across processes or persisted as calendar dates.

Fractional milliseconds preserve submillisecond precision; actual resolution and
whether system suspension counts as elapsed time are platform-dependent.
The function takes no arguments, needs no capability, and respects cancellation
and ordinary scalar-result limits. It is suitable for HTTP failures, database
queries, filesystem work, and automation:

```scr
$started = Clock::monotonicMs();
// Work to measure, including a try/catch around operations that may fail.
$elapsedMs = Clock::monotonicMs() - $started;
```

PHP mapping: this serves the elapsed-time role of
[`hrtime()`](https://www.php.net/manual/en/function.hrtime.php), using floating-point
milliseconds rather than PHP's nanosecond number or seconds/nanoseconds array.
`Date::now()` remains the wall-clock Unix-millisecond API for dates.

## Dates

| API | Behavior |
| --- | --- |
| `Date::now(): int` | Current wall-clock Unix **milliseconds**; may move backward when the system clock changes. Timezone selection changes display, not this instant. |
| `Date::format(timestamp: int, format: string, timezone: string optional): string` | Formats signed Unix milliseconds using [Chrono strftime directives](https://docs.rs/chrono/latest/chrono/format/strftime/). Omitted timezone means UTC; pass `"local"`, `"UTC"`, or an IANA name such as `"America/New_York"`. |
| `Date::parse(value): int` | Parses RFC 3339 with an explicit `Z` or numeric offset, returning UTC Unix milliseconds. Submillisecond precision is floored, including dates before 1970. |
| `Date::sleep(milliseconds): null` | Same nonnegative int duration, `sleep` permission, and cooperative cancellation as global `sleep`. Uses monotonic elapsed time. |

Timezone names and aliases are looked up case-insensitively in the bundled IANA
database. `"local"` is a case-sensitive selector for the host timezone. There is
no mutable script-wide default timezone. Named-zone rules are compiled into every
CLI build, including minimal builds: no external date command, downloaded library,
network connection, or system zoneinfo installation is needed for named zones.
Updating those rules requires updating the dependency data and rebuilding Scripta.

Local detection uses the process's `TZ` override when present, otherwise the OS
configuration. It may read local timezone configuration files on Unix or map the
Windows timezone setting to an IANA zone. Unknown/unavailable local configuration
raises `DateError` instead of falling back to UTC. Detection is cached by Jiff;
restart long-running hosts after changing timezone configuration. `Env::set`
changes the runtime's environment snapshot and child environments, not the running
process timezone. Pass a zone explicitly to select a different timezone per call.
Local detection needs host configuration, but does not use the internet.

Offsets are calculated for the supplied instant, including historical rules and
daylight-saving transitions. `%Z` prints its abbreviation, `%:z` prints a numeric
hour/minute offset, and `%::z` includes offset seconds for historical rules.
Abbreviations can be ambiguous; use IANA names to select zones. Include an offset
when displaying local times that can repeat during a daylight-saving transition.
A literal `Z` in the format is printed literally, so reserve it for UTC output.

Invalid dates, missing parse offsets, invalid format directives, unknown zones,
and timezone names that are empty, contain NUL, or exceed 255 UTF-8 bytes raise
`DateError`. A numeric offset string such as `"+05:30"` is not a zone selector;
use an IANA name. Wrong types/counts, including an explicit null timezone, raise
`TypeError`. Out-of-range timestamps raise `RangeError`: named/local timezone
conversion supports Jiff's timestamp range (years -9999 through 9999); omitted or
explicit UTC retains Chrono's existing wider range. Leap seconds remain unsupported.
`Date::parse` still requires RFC 3339 with an explicit offset; it does not guess
ambiguous or nonexistent local wall times. `Date::format` checks cancellation and
preflights its exact output against the logical value budget before allocating.

```scr
<?scripta
$now = Date::now();
echo Date::format($now, "%Y-%m-%dT%H:%M:%S%.3fZ") . "\n";
echo Date::format($now, "%Y-%m-%d %H:%M:%S %:z", "local") . "\n";
echo Date::format($now, "%Y-%m-%d %H:%M:%S %:z", "America/New_York") . "\n";
```

See the [timezone example](../examples/timezones.scripta). Calendar formatting
remains locale-independent, `Log` timestamps remain UTC, and `Clock::monotonicMs`
remains the elapsed-time API.

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

## URL and form encoding

These pure helpers require no network or filesystem capabilities. They operate
on UTF-8 text, preserve input collections, and check cancellation and result byte
limits (including nested pair slots) before allocating growing outputs.

| Function | Contract |
| --- | --- |
| `Url::encodeComponent(text: string): string` | Percent-encodes UTF-8 bytes, leaving only ASCII letters, digits, and `-._~` unchanged. Spaces become `%20`; literal `+` becomes `%2B`. |
| `Url::decodeComponent(text: string): string` | Decodes percent escapes once; literal `+` remains `+`. |
| `Url::buildQuery(parameters): string` | Encodes names/values and joins them with `&` and `=`, without a leading `?`. |
| `Url::parseQuery(text: string): list` | Parses a query into ordered two-string pairs, optionally removing one leading `?`. Literal `+` remains `+`. |
| `Form::encode(parameters): string` | Builds an `application/x-www-form-urlencoded` body. Spaces become `+`; ASCII letters, digits, and `*-._` remain unchanged. |
| `Form::decode(text: string): list` | Parses a form body into ordered two-string pairs. Converts raw `+` to space before percent-decoding; does not remove a leading `?`. |

Both builders accept a map of string keys and string values, or a list of
exactly two-string pairs. Map insertion order and list order are preserved;
pairs allow duplicate names. Integers, booleans, null, nested values, integer map
keys, and malformed pairs raise `TypeError`; use explicit conversions such as
`str(2)`. Empty maps/lists produce an empty string.

Both parsers split on `&`, ignore empty segments, and split each remaining
segment at its first `=` before decoding. Missing `=` means an empty value;
empty names/values and duplicate names are retained. Empty input returns an
empty list. Inputs are query strings or form bodies, not complete URLs. Neither
parser treats bracketed names as nested structures. Use `Form::decode` for
queries produced using HTML form conventions.

All encoders emit uppercase hexadecimal escapes. Decoders accept either hex
case, reject incomplete/nonhex escapes, and reject invalid decoded UTF-8 with
`EncodingError`. They never recursively decode: `%2526` becomes `%26`. Encoders
expect raw text: an existing `%` is encoded again. Component encoding applies to
one query name/value or individual path segment, not an entire URL, hostname, or
filesystem path. The percent profile uses the unreserved set from
[RFC 3986](https://www.rfc-editor.org/rfc/rfc3986#section-2.3); form encoding uses
the [WHATWG form safe set](https://url.spec.whatwg.org/#application/x-www-form-urlencoded).
Decoding here is deliberately strict about malformed escapes and UTF-8.

```scr
$query = Url::buildQuery(["q" => "café & tea", "page" => str(2)]);
$url = "https://example.com/search?" . $query;
// https://example.com/search?q=caf%C3%A9%20%26%20tea&page=2
$pairs = Url::parseQuery("tag=rust&tag=scripting");
// [["tag", "rust"], ["tag", "scripting"]]
$body = Form::encode(["name" => "Jane Doe", "message" => "A+B"]);
// name=Jane+Doe&message=A%2BB
```

For HTTP form submission, pass `$body` to `Http::post` and explicitly set the
`Content-Type` header to `application/x-www-form-urlencoded`. The local-only
[encoding example](../examples/url-encoding.scripta) runs without making requests.

PHP mapping: `rawurlencode` / `rawurldecode` correspond to
`Url::encodeComponent` / `Url::decodeComponent`; `http_build_query` corresponds
to `Url::buildQuery` for the percent profile or `Form::encode` for form encoding.
Scripta requires string parameters and does not implement PHP's nested-array or
implicit scalar conversion rules. `Form::decode` preserves pairs instead of
`parse_str`-style variable, name, or nesting transformations.

## Web requests

The `scripta serve` host explicitly registers nine `Web` and ten `Session` functions per
request. They are absent from ordinary CLI execution and from the default export
count above. Embedders can use `scr_stdlib::web::register` with a bounded request
and response-header ceiling. See [web hosting](web-servers.md) for the server,
worker lifecycle, resource ceilings, and reverse-proxy configuration.

| Function | Behavior |
| --- | --- |
| `Web::request(): WebRequest` | Returns this request's read-only object. |
| `Web::status(code: int): null` | Sets status 200–599; defaults to 200. Informational responses are not supported. |
| `Web::header(name: string, value: string): null` | Replaces all headers with this case-insensitive name. Does not change status automatically. |
| `Web::addHeader(name: string, value: string): null` | Appends a header, preserving repeated values such as `Set-Cookie`. |
| `Web::redirect(location: string, status: int = 302): null` | Replaces `Location` and sets 301, 302, 303, 307, or 308. Does not stop execution; follow with `exit 0`. |
| `Web::cookie(name: string, value: string, options = []): null` | Appends a validated Set-Cookie header; UTF-8 values are percent-encoded. |
| `Web::deleteCookie(name: string, options = []): null` | Appends a deletion cookie for the chosen path/domain. |
| `Web::download(source: ByteStream, filename: string): int` | Streams a readable file from its current position as an attachment. Returns bytes written (zero for HEAD). Requires no prior body output or active application buffers. See [file transfers](#upload-and-download-contract). |
| `Web::write(value: string or bytes): null` | Appends UTF-8 or arbitrary bytes to the bounded response body. `echo` also writes the body, with its usual strict types. |

`WebRequest` exposes `method`, `path`, and `queryString` string properties, plus
`bodySize: int` (the measured raw body byte length). `path`
retains percent encoding and excludes the query; use explicit URL decoding when
needed. It also exposes these read-only host-resolved properties:

| Property | Type / meaning |
| --- | --- |
| `requestId` | String or null: host-generated correlation ID. `scripta serve` returns the same value in `X-Request-ID` and ignores caller-supplied IDs. |
| `peerAddress` | String or null: actual TCP peer endpoint including port (`127.0.0.1:1234` or `[::1]:1234`). |
| `clientAddress` | String or null: resolved client IP without port. Defaults to the peer IP; proxy resolution requires explicit host trust. |
| `scheme` | String or null: resolved `http`/`https` under `scripta serve`, with direct connections using `http`. |
| `host` | String or null: normalized resolved hostname/authority including an explicit port when supplied. |

The HTTP supervisor resolves these once before dispatch. Raw headers remain
unchanged and must not be substituted for these properties when identifying a
client or building an external URL. See [proxy trust and host validation](web-servers.md#trusted-proxy-metadata-and-allowed-hosts)
for exact chain, syntax, and allowlist rules. Hostname metadata alone is not an
allowlist or authentication policy. Rust embeddings supply `Request.connection`
using `web::ConnectionInfo` and `Request.request_id`; `Default::default()` for
connection metadata and `request_id: None` leave these properties unknown
(null), and the standard library never infers them from supplied headers.
Connection metadata and request-ID dynamic strings count toward the request value
budget. See [host logs and health checks](web-servers.md#logs-and-health-checks)
for ID lifecycle and privacy rules.
These properties do not add registered functions: export counts stay 207 default
functions plus 19 request-context functions.

It has these methods:

| Method | Behavior |
| --- | --- |
| `header(name: string): string or null` | First matching value, case-insensitive; null when absent. |
| `headers(name: string): list` | All matching string values in received order, or an empty list. |
| `cookie(name: string): string or null` | First matching case-sensitive cookie name, percent-decoded as UTF-8, or null. Literal `+` stays `+`. |
| `cookies(): list` | All decoded `[name, value]` string pairs, retaining duplicates and order across Cookie fields. |
| `query(): list` | Decoded `[key, value]` string pairs, preserving duplicates and order. Uses form-style `+` as space. |
| `bodyStream(): ByteStream` | Fresh read-only raw-body stream with an independent cursor. No ambient filesystem-read capability is needed. |
| `bytes(): bytes` | Raw, fully received bounded body. |
| `text(): string` | Body as strict UTF-8; invalid encoding raises `EncodingError`. |
| `json(): any` | JSON body using existing `Json::decode` semantics. Requires exactly one Content-Type with `application/json` or `application/*+json`. |
| `form(): list` | Text `[name, value]` pairs from URL-encoded or host-parsed multipart forms, retaining duplicates and received order. |
| `uploads(): list` | Request-owned `UploadedFile` objects in received file order; empty for other body types. |

Query/form decoding uses the strict existing form codec: malformed percent
escapes and invalid UTF-8 raise `EncodingError`; duplicate names remain separate
pairs, numeric-looking keys remain strings, and no bracket/nested-key expansion
occurs. Malformed JSON raises `JsonError`. Content-Type parameters are accepted;
body text must still be UTF-8. Missing/ambiguous or unsupported media types raise
`WebError`. Calling these helpers does not consume the body; repeated calls work.
For `multipart/form-data`, the HTTP host parses the complete bounded request
before invoking the script. Text fields must contain UTF-8; their names remain
literal strings (no PHP bracket expansion). File contents stay binary. Submitted
names/filenames use UTF-8 disposition parameters as parsed by Multer; filenames
are opaque, with no percent decoding, path normalization, or MIME verification.
RFC 5987 `filename*` parameters are not interpreted; clients must use the
RFC 7578 `filename` parameter. Transfer encodings, nested multipart text fields,
and non-UTF-8 text charsets are rejected. An empty submitted filename and empty
body means “no selection” and is omitted from `uploads()`; a named zero-byte file
is retained. Empty selections still count toward the file limit. A nonempty body
with an empty filename is rejected. Text fields and files retain their respective
arrival order; the API does not reconstruct an interleaved stream of both kinds.

`UploadedFile` exposes read-only `fieldName: string`, `filename: string`,
`mediaType: string or null`, `size: int`, and `moved: bool`. Filename and advertised
media type are **untrusted metadata**, never destination paths or content checks.
`stream(): ByteStream` opens a read-only stream for inspection/forwarding before
saving, without revealing a path or requiring `filesystem.read`. Each cursor is
independent and shares the registered stream quota. New streams after a successful
move are rejected; previously opened streams remain request-owned until close or
request completion.
`moveTo(destination: string): null` requires an explicit application-chosen path,
`filesystem.write`, and the host filesystem policy. It rejects every existing
destination, including symlinks, directories, and read-only files. There is no
overwrite option. No general filesystem read capability is needed to save a
request's upload. A bounded, cancellation-aware copy to a private staging file
beside the destination supports cross-filesystem saves; publication then refuses
to replace an existing destination. Failure leaves the destination and the
handle's `moved` flag unchanged. Success marks all aliases of the handle consumed;
a second save raises `UploadError`. Saves are immediate filesystem effects and
remain after a later script error, just like ordinary filesystem writes.

Upload handles and captured upload methods belong to one runtime/request and
expire at request completion. `UploadError` rejects use from another runtime or
after expiry. Temporary paths are not properties of the objects. Raw multipart
`bytes()`/`text()` still return the original body, read on demand from the spool,
and preflight the full returned value against `max_value_bytes`; invalid UTF-8
raises `EncodingError`. Other multipart helpers copy only bounded metadata/text.
The logical value budget counts metadata and collection slots, rather than the
external file payload, as with filesystem resources. The host separately limits
temporary disk and total incoming bytes; see [web server limits](web-servers.md).

Rust hosts can use `web::register_multipart`, trusted `web::uploads::Multipart`
descriptors, a `FilesystemPolicy`, and a supervisor-owned staging callback. Keep
the returned `RequestResources` guard until execution ends, then drop it before
removing storage. Ordinary `web::register` has no multipart parser backend; the
`scr-web` host supplies the descriptors. Cookie and session behavior follows below.

Invalid statuses, redirect destinations, headers, or response-header ceilings
raise `WebError` without changing the previous response metadata. Header names
are normalized to lowercase; invalid values, including CR/LF injection, are
rejected. Content-Length and hop-by-hop transport headers are host-managed and
cannot be supplied by scripts. Redirects accept relative URIs or absolute HTTP(S)
URLs without credentials; scheme-relative URLs and backslashes are rejected.
Applications must still validate user-controlled destinations to prevent unwanted
external redirects. Prefer 303 for redirecting a successful POST to a GET page.

Web output is buffered until execution finishes unless `flush()` explicitly
requests transmission. Application buffers are independent: `ob_flush()` drains
the top application buffer into its parent, without committing HTTP headers.
Before the first transport flush, headers may change after `echo`; uncaught
errors, nonzero exits, and output overflow discard pending output and return a
generic 500. The first `flush()` saves/closes the session and commits response
metadata, even with no pending body. Later status/header/cookie/redirect changes
raise `WebError`; later session operations raise `SessionError`. Once committed,
failures terminate the body and are logged; the response cannot become a 500.
Clients may receive a truncated chunked response. Saved session changes remain.

All generated body bytes share a cumulative limit, including output later
cleaned from an application buffer. Normal responses default to
`text/html; charset=utf-8`; set an explicit Content-Type for JSON, text, or bytes.
HEAD and no-body statuses are enforced by the host. These response-local APIs
grant no filesystem, process, or outgoing-network permissions. Native work checks
cancellation; returned values obey runtime value limits. See
[output buffering](#output-buffers-and-flushing) and [host behavior](web-servers.md).

```scr
<?scripta
$request = Web::request();
if ($request->path === "/old") {
    Web::redirect("/new", 302);
    exit 0;
}
Web::header("Content-Type", "application/json; charset=utf-8");
echo Json::encode(["method" => $request->method, "query" => $request->query()]);
```

## Cookies

`Web::cookie` and `Web::deleteCookie` use a strict options map (an empty list is
also accepted). Defaults are `path: "/"`, host-only (no `domain`), `secure: true`,
`httpOnly: true`, and `sameSite: "Lax"`. Omit lifetime options for a browser-session
cookie. Available options:

| Option | Type / bounds |
| --- | --- |
| `path` | ASCII absolute path, at most 1024 bytes, without controls, semicolons, or backslashes. |
| `domain` | ASCII DNS-style domain, at most 253 bytes; leading dots are removed and letters lowercased. Omit for host-only cookies. |
| `secure` | Bool; set false explicitly for cookies used over local plain HTTP. |
| `httpOnly` | Bool; false allows browser JavaScript to read this cookie. |
| `sameSite` | String Lax, Strict, or None (case-insensitive); None requires Secure. |
| `maxAge` | Int seconds, 0–34560000; Max-Age takes precedence over Expires in browsers. |
| `expires` | Int Unix seconds, 0–253402300799, formatted as a GMT HTTP date. |

Cookie names follow HTTP token rules and are limited to 256 ASCII bytes. Values
use the component codec, including percent encoding spaces, delimiters, and UTF-8;
there is no implicit conversion. Incoming values are percent-decoded once, with
literal plus signs preserved. Invalid percent escapes or UTF-8 raise
`EncodingError`. Malformed Cookie syntax raises `CookieError`. At most 256 received
cookie pairs and 4096 bytes per generated Set-Cookie field are allowed, in addition
to overall request/response header ceilings. Duplicate names remain separate;
`cookie(name)` returns the first. Cookies are untrusted client input.

Each call appends its own Set-Cookie field. Browser scope/replacement rules apply;
setting a response cookie does not modify the current request object. Deletion
sends an empty value, Max-Age=0, and a past Expires value; use the same path/domain
as the original cookie. Header/cookie failures leave earlier response headers
unchanged. Unknown options or invalid values/prefix rules raise `CookieError`;
normal signature errors remain `TypeError`. Secure/Host/Http cookie prefixes are
validated, including Secure, HttpOnly, Path=/, and no-Domain requirements where
applicable. Invalid generic response-header budgets raise `WebError`.

```scr
<?scripta
$request = Web::request();
$theme = $request->cookie("theme");
Web::cookie("theme", "dark", ["maxAge" => 3600]);
Web::deleteCookie("obsolete", ["path" => "/"]);
```

The Secure default assumes HTTPS at the browser, even when a reverse proxy talks
plain HTTP to Scripta. The host's session-cookie flag below affects session
cookies only; ordinary `Web::cookie` options remain explicit.

## Sessions

Sessions require the `web.session` capability, granted by the web host and absent
from ordinary runtime defaults. Embedders must grant it explicitly when registering
the Session module and prepare its dedicated storage configuration. This scoped
capability is independent of generic filesystem roots/read/write capabilities.
Sessions are server-side and shared by all workers through the host's dedicated
local directory. They require explicit `Session::start()`; endpoints that never
start a session do not allocate records or emit a session cookie. Only the
`SCRIPTA_SESSION` cookie supplies an ID; query/form fields are never session IDs.
That cookie name is reserved for the session runtime. Do not set it manually.

| Function | Behavior |
| --- | --- |
| `Session::start(): null` | Resume a known, unexpired session or generate a new random one. Idempotent while open. |
| `Session::id(): string` | Current opaque 64-character lowercase hexadecimal ID. |
| `Session::get(key: string, default: any = null): any` | Stored value, including stored null; default only when the key is absent. |
| `Session::has(key: string): bool` | Distinguishes an absent key from a stored null. |
| `Session::set(key: string, value: any): null` | Set/replace a value without moving an existing key's insertion position. |
| `Session::remove(key: string): bool` | Remove a key; return whether it existed. |
| `Session::all(): map` | Snapshot of all session fields in insertion order. |
| `Session::regenerate(): null` | Generate a fresh ID, preserving data; revoke the previous ID on commit. At most once per request. Call after successful authentication. |
| `Session::destroy(): null` | Stage deletion of server data and expiration of the session cookie. Further session access in this request fails. |
| `Session::close(): null` | Commit immediately and release the record lock. Further session access fails. Later request errors do not undo this explicit early commit. |

All operations other than start require an open, non-destroyed session. String
keys stay strings. Values may contain null, bool, int, finite float, UTF-8 string,
bytes, lists, and maps with typed integer/string keys. Storage preserves strict
number types, byte contents, typed keys, and insertion order. Resources and
callables are rejected. Nesting is limited to 32 levels including the session
root; both encoded storage bytes and logical value/collection bytes must fit the
host's per-session ceiling. Validation and failed set/remove operations preserve
the previous session values. Returned values obey normal runtime value budgets.

IDs contain 256 random bits from the OS random source. Unknown/invalid-format,
expired, or revoked IDs cannot select/create an attacker-chosen session: a new ID
is generated instead. Duplicate session cookies or malformed Cookie syntax cause
`SessionError` on start. Regeneration invalidates the old token on a successful
commit; destroying writes an empty revoked record and expires the cookie.

A per-session lock serializes concurrent requests from the same session to avoid
lost updates. Other sessions execute independently except for short directory
maintenance/commit locks. Lock waits check cancellation and have a 30-second
ceiling; the host request deadline can terminate a blocked worker sooner. The OS
releases its locks on exit, including abrupt worker death. Use close before slow
unrelated work if you no longer need session access.

Without close, the host commits only after script exit 0 and successful bounded
body generation. Uncaught script errors, nonzero exits, and output overflow before
that point discard changes. Storage is replaced atomically, and persistence
failure turns the response into a generic 500 before headers/body are sent.
Network failure or a deadline during the final commit/transmission can leave a
completed commit without a delivered response; HTTP delivery and disk storage
are not one transaction. Regeneration/commit errors do not expose a new token in
the generic error response.

Session responses use `Cache-Control: no-store`. The session cookie is host-only,
Path=/, HttpOnly, SameSite=Lax, and Secure by default, with Max-Age matching the
idle lifetime. Successful commits refresh the server expiry and browser lifetime. Persistent
expiry uses Unix milliseconds; host date/time adjustments affect expiry.
Session storage defaults to a private temporary directory, 30-minute idle expiry,
10000 records, and 64 KiB per session. Explicit host options can select persistent
storage and adjust limits; see [hosting options](web-servers.md#sessions-and-cookies).
A rotation can temporarily reserve one extra record; expired/revoked/uncommitted
records and abandoned staging files are reclaimed when creating new records.
Expired records are never accepted while awaiting reclamation.

```scr
<?scripta
Session::start();
$visits = Session::get("visits", 0) + 1;
Session::set("visits", $visits);
Session::close();
Web::header("Content-Type", "application/json; charset=utf-8");
echo Json::encode(["visits" => $visits]);
```

The file backend is for trusted applications on one host using local storage;
records are private plaintext, not encrypted. Persistent directories must be
outside the public document root, dedicated to one application, and accessible
only to its service account. Unix directories/files must be private, symlink
entries are rejected, and hard links are rejected on Unix. Windows relies on the
OS account's directory ACLs; configure private ACLs for persistent storage. This
does not grant scripts arbitrary filesystem access beyond existing policies.
Independent hosts without the same store do not share sessions; distributed
session backends are not implemented. Sessions do not supply authentication, authorization, or CSRF protection.
Use the [Password module](#passwords) for password hashing and verification. See the runnable
[session example](../examples/web-sessions.scripta).

## HTTP

`Http::get(url, options = [])`, `Http::post(url, body, options = [])`, and
`Http::request(method, url, options = [])` make synchronous HTTP/HTTPS requests.
URLs must be absolute, have a host, and contain no embedded user credentials.
POST's body accepts a string or bytes; JSON requests use `Json::encode(value)` explicitly
and supply `Content-Type: application/json`. Generic request bodies use the
`body` option, also accepting string or bytes. Supplying both POST's body argument and a body option is an error.
Methods are case-sensitive HTTP tokens; CONNECT tunnels are unsupported.

| Option | Default and behavior |
| --- | --- |
| `timeoutMs` | 30000; positive int within the platform clock range, covering the operation including redirects and body reads. |
| `headers` | Map of string header names to string values. Invalid names/newlines are rejected; Host, Content-Length, and Transfer-Encoding are managed by the client. |
| `maxBodyBytes` | 8388608 (8 MiB); nonnegative int, no higher than the host ceiling. Counts bytes before UTF-8 decoding. An oversized response is an error, never successful truncation. |
| `maxRedirects` | 5; nonnegative int, no higher than the host ceiling. Zero returns the original 3xx response without following it. |
| `body` | Empty body; string or bytes for `Http::request`. |
| `responseType` | `"text"` (default) or `"bytes"`. Text mode validates UTF-8 before returning; bytes mode retains arbitrary data. |

Unknown options and invalid values raise `TypeError` before connecting. Headers
are optional; omit `headers` or use `headers: []` for an empty set. Each request returns a read-only
`HttpResponse` after the complete bounded body has been read:

| Member | Behavior |
| --- | --- |
| `status: int` | HTTP status code. |
| `durationMs: float` | Read-only, nonnegative elapsed milliseconds through complete bounded body reception, including redirects. Fractional milliseconds are retained; the value is fixed when the transfer finishes. |
| `headers: map` | Lowercase names mapped to **lists of strings**, preserving repeated header values. Header ordering is unspecified. |
| `body: string or bytes` | Text in default mode; raw bytes when `responseType` is `"bytes"`. No charset conversion is performed. |
| `ok(): bool` | True for status 200–299, false otherwise. |
| `json()` | Decodes body using exactly `Json::decode` semantics, including line/column errors. Does not require a Content-Type header. |
| `text(): string` | Strictly decodes retained bytes as UTF-8; invalid bytes raise `EncodingError`. Repeated calls do not consume the body. |
| `bytes(): bytes` | Returns the retained raw body without decoding. Available in both response modes. |

`durationMs` starts after argument/request validation and before client setup,
covering DNS, connection/TLS setup, redirects, request upload, header capture,
and bounded body buffering through EOF. It stops before UTF-8 validation,
response-object construction, and later script processing such as `json()`.
It is available for text, bytes, empty bodies, and completed HTTP 4xx/5xx
responses. Reading it later does not increase its value.

Transport, encoding, body-limit, permission, and cancellation failures produce
no partial response. Measure attempted calls with `Clock::monotonicMs()` around
a try/catch to obtain elapsed time on failure. The
[HTTP timing example](../examples/http-timing.scripta) shows both APIs and runs
locally without a URL. Use the clock around `Http::download` or other operations
that do not return an `HttpResponse`. Detailed DNS/TLS/first-byte milestone maps
are not exposed by this API.

PHP mapping: `durationMs` serves the total-transfer role of
[`curl_getinfo()` total_time](https://www.php.net/manual/en/function.curl-getinfo.php),
expressed in milliseconds. It is not a complete cURL timing breakdown.

HTTP response storage includes raw bytes and headers in the runtime value budget.
Use `responseType: "bytes"` to receive a binary body; `json()` and `text()`
still require valid UTF-8 when called.

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

`Http::download(url: string, destination: string, options optional): map` streams
a GET response through a 64 KiB buffer into a temporary file beside the destination.
It returns `status`, `headers` (the same map shape as a response), and
`bytesWritten`. It requires `http.request` and `filesystem.write`, including host
filesystem policy checks on the destination and parent. Parent directories must
exist. All existing destinations, including symlinks, are refused with
`AlreadyExistsError`; overwrite is not supported. A race creating the destination
also refuses publication.

Only 2xx responses are published; other statuses raise `HttpStatusError`.
A completed staging file is synchronized, closed, and published without overwriting
an existing name. Transfer, timeout, cancellation, limit, and filesystem failures
remove the temporary file and leave the destination absent. Cleanup errors are
included in the exception. Download output is not decoded, decompressed, or
extracted. It accepts the HTTP options above except `body`; `responseType` has no
effect on the downloaded bytes. Header/status metadata must fit the runtime value
budget, but the file itself need not. The default **8 MiB HTTP body ceiling also
applies to downloads**; embedders must raise the host ceiling to allow larger
archives, and scripts can only lower that ceiling with `maxBodyBytes`.
Cancellation is checked between body chunks; blocking socket work remains bounded
by its timeout.

Requests require `http.request`, enabled by default. Hosts can remove that
capability or use `StdlibOptions { http: HttpConfig { timeout, max_response_bytes,
max_redirects }, ..Default::default() }` with `scr_stdlib::http::HttpConfig`.
Per-call timeout overrides must remain positive; body/redirect limits cannot
exceed host ceilings. HTTP responses never automatically enter logs.

```scr
<?scripta
$response = Http::get("https://example.com/api/status");
if (!$response->ok()) throw "API request failed";
$data = $response->json();
echo $data["status"] . "\n";
```

See [local script loading](language-spec.md#local-script-loading) for reusable
helpers and configuration files. HTTP never acts as a remote module loader.


## Distribution and resource behavior

Every module above is statically linked into scripta; there is no separate standard
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

## Output buffers and flushing

These eleven native functions are available in CLI scripts and web requests.
They do not add syntax. Output buffers are binary: retrieval returns `bytes`,
with `Bytes::decodeUtf8` available for explicit strict text decoding.

| Function | Behavior |
| --- | --- |
| `ob_start(): null` | Push an empty application buffer. |
| `ob_flush(): null` | Drain the top buffer into its parent (or host writer), leaving it active and empty. |
| `ob_end_flush(): null` | Drain and remove the top buffer. |
| `ob_clean(): null` | Discard the top buffer's bytes, keeping it active. |
| `ob_end_clean(): null` | Discard and remove the top buffer. |
| `ob_get_contents(): bytes or null` | Copy the top buffer; null when no buffer exists. |
| `ob_get_clean(): bytes` | Copy then discard/remove the top buffer. |
| `ob_get_flush(): bytes` | Copy then drain/remove the top buffer. |
| `ob_get_length(): int or null` | Top buffer's byte length, or null without a buffer. |
| `ob_get_level(): int` | Number of active application buffers. |
| `flush(): null` | Request a flush of the underlying host writer. Does not drain application buffers. |

Drain/clean operations requiring a buffer raise `OutputError` when none is
active. Signatures take no arguments; callbacks, transformations, implicit
flushing, and threshold/chunk-size options are not supported. Retrieval checks
its complete result budget before copying or removing/draining a buffer; failed
result-budget or result-allocation checks preserve the buffer and assignment
destination. A writer failure during drain can already have emitted partial data
and prevents successful finalization. Embedded hosts set
`RuntimeOptions.max_output_buffer_bytes` (default 8 MiB, hard ceiling 64 MiB)
and `max_output_buffers` (default 32, hard ceiling 128). Pending bytes are counted
across all levels. Cleaning releases storage; draining does not count the same
bytes as new output. `max_output_bytes` optionally caps cumulative generated
bytes, including bytes subsequently discarded. The CLI leaves that cumulative
limit unset; `scripta serve` sets it to `--max-response-bytes` and caps pending
application buffers at the smaller of that limit and 8 MiB.

Buffers are local to one execution, shared by functions and required modules.
On normal completion (including `exit`), remaining buffers drain from inner to
outer without requesting a transport flush. Uncaught exceptions, cancellation,
and fatal execution limits discard remaining application buffers. Output already
sent to a host writer cannot be rolled back. Writer failures and pending/total
output-limit failures remain sticky even if native errors are caught; they prevent
successful finalization. Nesting/no-active-buffer errors can be caught normally.
Output flushing uses the caller's writer and does not flush the diagnostic stream.

For web responses, `ob_flush()` alone does not send the HTTP response. Use:

```scr
<?scripta
Web::header("Content-Type", "text/plain; charset=utf-8");
ob_start();
echo "Starting\n";
ob_flush();
flush();
// Headers and sessions are now closed to further changes.
echo "Finished\n";
ob_end_flush();
```

Web transport uses bounded body chunks with backpressure and the request deadline.
A successful `flush()` means the host accepted the pending chunks; proxy,
compression, TCP, and browser buffering may delay visible output. It does not
acknowledge that a remote client consumed the bytes. See
[the streaming example](../examples/web-streaming.scripta).

## General streams

`Stream` supplies binary I/O without changing the existing `FileStream` line
reader, `File::openLines`, `File::readLine`, or `File::close` APIs. New handles are
`ByteStream` (file, HTTP response, connected TCP) or `DatagramSocket` (connected
UDP). Handles are read-only native objects with `closed`, `readable`, `writable`,
`seekable`, `status` (HTTP status or null), and `headers` (HTTP header map or empty
map). Mode properties describe the configured mode even after close.

| Function | Contract |
| --- | --- |
| `Stream::open(uri: string, mode: string optional, options = []): ByteStream or DatagramSocket` | Opens a file, HTTP response body, or connected transport. Default mode is `r` for files/HTTP, `rw` for TCP/UDP. |
| `Stream::request(method: string, url: string, source: ByteStream, options = []): ByteStream` | Sends an incrementally consumed **readable file or request-input stream** as an HTTP request body, returning a readable HTTP response stream. Source is not closed on success. |
| `Stream::read(stream: ByteStream, length: int): bytes or null` | Reads up to the requested count; short reads are valid. Positive reads return null only at EOF. Zero length returns empty bytes without consuming input or changing EOF. |
| `Stream::write(stream: ByteStream, data: bytes): int` | Writes up to one bounded chunk; returns the actual count, possibly short. Call again for unwritten bytes. Strings require `Bytes::encodeUtf8`. |
| `Stream::flush(stream): null` | Flushes a writable file/TCP handle; UDP is a no-op. Read-only HTTP/input handles reject it. Does not flush script output buffers or guarantee durable disk storage. |
| `Stream::close(stream): null` | Closes shared handle state. Idempotent for an owned handle; aliases also become closed. Requires ownership, but no I/O capability for cleanup. |
| `Stream::seek(stream: ByteStream, offset: int, origin: string = "start"): int` | File-only seek; `start` uses nonnegative offsets, `current`/`end` accept signed offsets. Returns the new position and clears EOF. |
| `Stream::tell(stream: ByteStream): int` | File-only current position; does not change EOF. |
| `Stream::eof(stream: ByteStream): bool` | True after a positive read observes EOF. A short nonempty read does not establish EOF. |
| `Stream::copy(source: ByteStream, destination: ByteStream, maxBytes: int): int` | Copies up to a bounded count using at most 64 KiB chunks, completing short writes. Rejects the same handle. Validates count against both handles' remaining transfer budgets before I/O. |
| `Stream::send(socket: DatagramSocket, data: bytes): int` | Sends exactly one UDP datagram, including a zero-length packet. |
| `Stream::receive(socket: DatagramSocket, length: int): map` | Receives exactly one packet and returns `data: bytes`, `address: string` (source IP), `port: int`, and `truncated: bool`. Keeps at most length bytes and discards the remainder of that packet. Empty packets remain ordinary results. |

### Wrappers and file modes

Bare paths use the native platform's path rules and the runtime working
directory. `file:///absolute/path` explicitly percent-decodes a UTF-8 local path;
use `file:///C:/path` on Windows. Literal plus stays plus. File wrapper URLs have
no authority, query, or fragment; remote/UNC file URLs are unsupported. Bare
native UNC paths retain the existing filesystem path rules. Unrecognized
`scheme://` wrappers raise `StreamError`. HTTP/TCP/UDP wrappers use lowercase
scheme spellings. TCP/UDP endpoints require **literal IP addresses and ports
1–65535**, for example `tcp://127.0.0.1:9000` or `udp://[::1]:9000`; hostname
resolution is not implicit. See [low-level sockets](#low-level-sockets) for
listeners, binds, readiness, and supported socket configuration. Connected UDP binds an ephemeral wildcard local port.

| File mode | Read | Write | Opening behavior |
| --- | --- | --- | --- |
| `r` | Yes | No | Existing file; no mutation. |
| `r+` | Yes | Yes | Existing file; preserves contents. |
| `w`, `w+` | Only `w+` | Yes | Create or truncate after validation. |
| `a`, `a+` | Only `a+` | Yes | Create if absent; every write appends even after seek. |
| `x`, `x+` | Only `x+` | Yes | Exclusive creation; refuses any existing destination. |

Files must be regular and cannot be final symlinks. Write modes reject read-only
files even if the OS account could bypass their read-only flag. Read/write modes
require both filesystem capabilities and the corresponding `FilesystemPolicy`
flags/roots at open. Later operations check their capabilities; the opened
descriptor retains its original authority if its path is renamed. Validation and
result-size checks run before file creation/truncation. Successful opening in `w`
mode is itself a truncation; subsequent failure does not undo it. Writes and copy
are incremental effects, with no atomic rollback after partial I/O. Use `x` to
refuse overwrite and `File::writeAtomic` for the separate UTF-8 atomic replacement
contract. `seek` does not reset transfer accounting. Filesystem blocking calls
cannot be forcibly interrupted by an in-process runtime.

### HTTP and transport policy

HTTP/HTTPS `open` accepts only `r`, performs a GET, and returns after headers
without materializing the response body. Status errors remain readable responses,
as with `Http::request`. Response `headers` preserve repeated values as lists.
`Stream::request` supports request methods except CONNECT and consumes a readable
file source from its current offset. Request-owned raw-body/upload input streams are also accepted; they contain
already-received data. Network response/TCP handles cannot be used
as upload sources, so pulling one connection cannot bypass
another request's timeout. Failed uploads may consume or close the source and
can have remote effects. Upload bodies are not replayed: redirects are disabled
for uploads, `maxRedirects` must be zero if supplied, and a 3xx response is returned
for application handling. GET response streams keep the existing bounded
redirect policy, verified Rustls TLS, HTTPS downgrade prevention, managed framing,
credential-free absolute URL validation, and disabled implicit proxies. There is
no option to disable certificate verification.

Files require `filesystem.read`/`filesystem.write`; HTTP requires `http.request`.
TCP requires `network.connect`. UDP requires both `network.connect` and
`network.datagram`. These outgoing capabilities are enabled in the default
trusted-automation permission set and can be removed by an embedding host. This
is not an OS sandbox. UDP is not a byte stream: generic read/write/copy/seek APIs
reject it. UDP send payloads and requested receive lengths are at most 65507 bytes;
receive uses a bounded 64 KiB working buffer to measure the full incoming packet,
charges all received bytes (including discarded bytes), and reports truncation.
There is no UDP EOF; empty packets and timeouts are distinct.

### Limits, deadlines, and lifecycle

Options are a map or empty list. `maxBytes` can only lower the handle's host
transfer ceiling. Each handle counts bytes read **plus** bytes written, including
repeated reads after seeks and full received datagrams. Exactly at the read
ceiling, a subsequent positive read probes at most one byte: EOF succeeds, while
extra data closes the stream with `StreamLimitError`. Failed reads never assign
partial results. Write requests exceeding the remaining budget fail before I/O.
Copy conservatively requires its maximum count to fit both remaining budgets,
even if EOF could occur sooner. Transfers between two handles count against each
handle separately. Zero-length operations still validate ownership and mode.

`timeoutMs` applies to network streams only and may only lower the host timeout.
File opens reject it. HTTP options also accept the existing validated `headers`,
`maxBodyBytes`, and `maxRedirects`; body/framing/response-type options are not
accepted. HTTP response ceilings can only be lowered, and the smaller of the
stream and HTTP host limits applies. An HTTP deadline starts when the request
starts and continues through later body reads, including time spent between
script calls. Socket deadlines apply per operation. Socket read/write/receive
waits poll cancellation at intervals up to 50 ms. TCP connection setup and HTTP
backend/DNS/TLS waits remain bounded backend operations; cancellation is checked
before/after calls and between body chunks. Web worker supervision supplies an
outer process deadline for blocked native work.

`StdlibOptions.streams: StreamConfig` defaults to 64 live handles, 65536 bytes per
chunk, 67108864 transfer bytes per handle, and 30 seconds per network operation.
Host ceilings are 1024 handles, 1 MiB per chunk, 1 GiB transfer bytes, and one hour.
The HTTP host's default 8 MiB response ceiling still applies. Pending response
bytes are not retained in the native object; its logical payload reports metadata.
`read` preflights the **requested maximum result**, even when a shorter read might
fit. UDP similarly preflights payload/map slots plus a maximum-length IPv6 address.
This is a logical value budget, not a process heap cap.

Handles require their actual native type and original runtime identity; spoofed
or foreign handles are rejected. Close is shared and idempotent. I/O errors,
timeouts, transfer overrun during read, and in-operation cancellation close the
handle to avoid resuming an ambiguous transfer. Type/range/budget/permission
validation errors leave it usable. Dropping the last handle reference closes the
descriptor and releases the live-handle slot. Normal runtime teardown releases
script-owned handles; host-retained values stay owned by the host until dropped.
Fatal cancellation can bypass script `finally`, so cleanup uses Rust RAII.

See [binary copying](../examples/binary-streams.scripta) and
[streamed HTTP uploads](../examples/http-stream-upload.scripta).

## Upload and download contract

The host automatically parses multipart metadata into `$request->uploads()`;
applications do not register temporary paths. `UploadedFile` handles are bound to
the originating request/runtime and cannot be constructed from a script path.
`moveTo()` checks that handle, its lifetime, and destination policy before copying
through supervisor-owned staging and publishing without overwrite. Temporary
input is removed on normal completion or after a failed/timed-out worker is
stopped. Submitted filenames and media hints remain untrusted. Use `stream()`
to inspect binary signatures or application formats before saving; the API does
not claim a client's MIME string is verified content.

| PHP capability | Scripta equivalent / policy |
| --- | --- |
| `$_FILES` original name, MIME hint, size | `UploadedFile.filename`, `.mediaType`, `.size`, plus `.fieldName`; repeated names remain ordered handles. |
| `$_FILES` temporary path | Private host storage, deliberately absent from script properties. Use `.stream()` to inspect contents. |
| `$_FILES` per-file upload error | Malformed/oversized/unsupported requests are rejected before application execution (400/413/415); aggregate spool exhaustion returns 503. No partially accepted upload handles are presented. Empty browser selections are omitted. |
| `move_uploaded_file()` | `$upload->moveTo($applicationDestination)`, requiring the original live native handle and refusing overwrite. |
| `php://input` | `$request->bodyStream()` for bounded binary reads/copies; `$request->bytes()`/`text()` materialize the full body only under value limits. |

PHP documents its upload metadata, client-supplied MIME hints, temporary-file
cleanup, and move verification in the [upload manual](https://www.php.net/manual/en/features.file-upload.post-method.php)
and [move_uploaded_file reference](https://www.php.net/manual/en/function.move-uploaded-file.php).
Its [input wrapper reference](https://www.php.net/manual/en/wrappers.php.php)
describes `php://input`, including multipart configuration caveats. Scripta uses
explicit request objects rather than introducing superglobals or PHP wrapper
spellings. Raw bytes remain available for both ordinary and multipart requests.

The supervisor spools nonempty raw bodies before dispatch rather than copying
large binary payloads through worker IPC. Request streams read the fully received
spool in bounded chunks; scripts do not execute concurrently with an unfinished
client upload. All bodies retain the request byte/time limits and share the host
spool disk budget. Empty bodies need no spool. Raw input is not an `UploadedFile`;
applications choose a destination explicitly using a writable stream and normal
filesystem policy. Every input cursor shares the configured stream-handle quota.
Request completion closes input descriptors and releases their quota, even when
Rust hosts retain a stream value. Expired or foreign request streams reject I/O.

`Web::download` requires an actual readable file stream, a basename of 1–255
UTF-8 bytes without path separators/control characters, and an unused response
body with no active output buffer. It preflights the known remaining file size
against source-transfer and output budgets before committing headers. It supplies
`application/octet-stream`, `Content-Disposition: attachment` with a quoted ASCII
fallback and encoded UTF-8 `filename*`, `X-Content-Type-Options: nosniff`, and
`Accept-Ranges: none`. Source filesystem permissions were checked at open and are
checked again for reads. Sessions close before the first transmission. Transfer
uses bounded chunks and flush acknowledgements/backpressure; failures after
commit terminate the response. Call `exit 0` afterward to avoid appending other
application output. The source remains caller-owned and should be closed in
`finally`. HEAD sends metadata without reading the payload. This is a whole-file
transfer from the current position; range/resumable responses are not advertised.
Concurrent source-file mutation is not a snapshot or locking guarantee.

See the [raw-upload/download example](../examples/web-transfers.scripta), the
[multipart form example](../examples/web-uploads.scripta), and outgoing
[`Http::download`](#http), which retains exclusive staged publication and cleanup
rather than overwriting an existing destination.

## Low-level sockets

Socket operations share `ByteStream`/`DatagramSocket` handles, identity checks,
closed state, cumulative byte limits, and the registered `StreamConfig` handle
quota. `SocketListener` is a separate read-only handle type. Existing connected
`Stream::open("tcp://...")`/UDP adapters use the same safe readiness backend.
Only IPv4/IPv6 literal addresses are supported; ports are checked integers.
There are no privileged raw IP, Unix-domain, WebSocket, or application event-loop
APIs. Applications can build protocols and scheduling in script code.

| Function | Contract |
| --- | --- |
| `Socket::connect(address: string, port: int, options = []): ByteStream` | Connects TCP to a literal IP and port 1–65535. Handshake is a bounded blocking backend call, then I/O honors the logical blocking mode. |
| `Socket::listen(address: string, port: int, options = []): SocketListener` | Binds and listens in one operation; port 0 chooses an ephemeral port. |
| `Socket::accept(listener: SocketListener): ByteStream` | Accepts at most one connection. Reserves a handle slot before removing a connection from the queue. Accepted sockets inherit logical blocking mode, timeout, and transfer ceiling. |
| `Socket::bindDatagram(address: string, port: int, options = []): DatagramSocket` | Binds unconnected UDP; port 0 is allowed. |
| `Socket::read(socket: ByteStream, length: int): bytes or null` | TCP read with the same binary/short-read/EOF rules as `Stream::read`. |
| `Socket::write(socket: ByteStream, data: bytes): int` | TCP write returning actual bytes written; finish short writes explicitly. |
| `Socket::receive(socket: DatagramSocket, length: int): map` | One UDP packet, preserving empty packets and reporting sender/truncation; same result as `Stream::receive`. |
| `Socket::sendTo(socket: DatagramSocket, data: bytes, address: string, port: int): int` | One packet to a literal peer from an unconnected bound UDP socket. Connected UDP uses `Stream::send`. |
| `Socket::shutdown(socket: ByteStream, direction: string): null` | TCP half/full shutdown: `read`, `write`, or `both`. Does not close the handle or release its slot. |
| `Socket::close(socket): null` | Shared, idempotent close for TCP/UDP/listeners; no I/O capability needed for cleanup. |
| `Socket::setBlocking(socket, blocking: bool): null` | Sets logical blocking behavior, shared by aliases. |
| `Socket::setTimeout(socket, milliseconds: int): null` | Sets a positive operation timeout within the host ceiling. |
| `Socket::poll(read: list, write: list, milliseconds: int): map` | Returns `readable`/`writable` lists of ready handles; unique results preserve each input list's order. Zero timeout probes immediately; EOF/error readiness is included. Requires at least one socket. |
| `Socket::localAddress(socket): map` | `address` and `port` of the bound endpoint. |
| `Socket::peerAddress(socket): map` | Connected TCP/UDP peer; errors for listeners and unconnected UDP. |
| `Socket::setOption(socket, name: string, value): null` | Supported runtime options below; strict types. |
| `Socket::getOption(socket, name: string): bool or int` | Returns the OS-reported option, which may differ from the requested buffer size. |

Open options accept `timeoutMs`/`maxBytes` as lower host ceilings and
`blocking: bool` (default true). Connect also accepts `noDelay: bool` (false).
Listen accepts `backlog: int` (1–4096, default 128), `reuseAddress: bool` (false),
and `ipv6Only: bool` (true, IPv6 only). Bind-datagram accepts the latter two.
Options not applicable to that operation/family are rejected before binding.
`reuseAddress: true` is explicitly unsupported on Windows because reuse semantics
differ; other option availability/errors follow the OS. Runtime `noDelay` and
`keepAlive` require TCP and bool values. `receiveBufferBytes`/`sendBufferBytes`
accept ints 1024–4194304; OS rounding/limits are reported by `getOption`.
Creation-only options cannot be changed through `setOption`.

Blocking read/write/accept/receive waits use Mio OS readiness and check
cancellation at intervals up to 50 ms. Nonblocking operations raise
`WouldBlockError` when not ready, leaving handles open; they do not return EOF or
an empty UDP packet. Poll bounds list lengths to the host handle ceiling, checks
result budgets before waiting, and observes cancellation. Readiness is advisory:
retry I/O and handle `WouldBlockError` after a poll. File/HTTP/request-input handles
cannot be polled or passed through `Socket` I/O functions.

TCP client access requires `network.connect`. Listen/accept and accepted TCP
connections require `network.listen`. Bound UDP requires `network.bind` plus
`network.datagram`; sending to a new peer additionally requires `network.connect`.
Default trusted CLI automation grants these capabilities. The web request worker
removes `network.listen` and `network.bind`; embedders choose their own permission
sets. These are cooperative capabilities, not an OS sandbox. Backlog and combined
live-handle limits bound pending/accepted resources; each accept creates one
handle. Long-running servers should also choose execution limits appropriate to
the host. No unlimited accept batch or unbounded user-space queue is provided.

Stream data errors follow the stream lifecycle rules (fatal I/O/timeouts close,
`WouldBlockError` does not). Listener accept timeout/errors preserve the listener,
and option/address/shutdown failures leave the handle available for cleanup.
UDP send-to errors report packet failure without a successful byte count. Closing
the last reference/normal runtime teardown releases descriptors through RAII.
Mio supplies safe cross-platform OS polling; Socket2 supplies safe bind/listen and
options. Both existing dependency versions support Rust 1.85; no unsafe Rust is
introduced in the workspace. OS-specific behavior still requires native platform
validation. See the [one-connection echo server](../examples/socket-echo.scripta).
