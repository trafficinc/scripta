# Scripta language specification

Scripta is a PHP-inspired systems scripting language, not a PHP implementation.
Its executable is `scr`, its source extension is `.scr`, and its implementation
uses stable Rust, edition 2024. Linux is the primary target, followed by macOS
and Windows.

## Final language decisions

| Feature | Decision |
| --- | --- |
| Language / executable / extension | Scripta / `scr` / `.scr` |
| Opening tag | Required `<?scr` |
| Closing tag | Optional `?>` |
| Execution | Synchronous, sequential, top to bottom |
| Variables | `$name` |
| Statements / blocks | Semicolons / curly braces |
| Concatenation | `.` with string operands only |
| Output | `echo "text";` and `echo("text");` |
| Types | Dynamic with optional annotations |
| Coercion | Strict; no implicit conversions |
| Equality | Value and type must both match |
| Collections | Separate list and map types |
| Errors | Explicit exceptions and useful diagnostics |
| Implementation / platforms | Rust / Linux first, then macOS and Windows |

Optional type annotations on function parameters and return values are checked
at call boundaries. Variable annotation syntax is deferred. Code blocks use `{`
and `}`. The two output spellings are
`echo "text";` and `echo("text");`. All conversions are strict and explicit.

These decisions define the target language; the implementation status below
describes the implemented grammar and runtime semantics.

## Implemented subset

Source is UTF-8. An optional BOM may appear at byte zero, followed by an optional
first-line Unix shebang (`#!/usr/bin/env scr`). Whitespace may precede the required
`<?scr` tag; executable content and comments may not precede it. An empty script
containing only the opening tag is valid. The tag may be immediately followed by
whitespace, comments, or punctuation, but not an identifier continuation
(`<?script` is invalid). A closing `?>` permits only whitespace afterward, even
excluding comments. Scripta never emits surrounding text as a template.

Whitespace, `//` and `#` line comments, and non-nesting `/* ... */` block comments
are ignored between tokens. Tags inside comments or strings are inert. A block
comment must close. Line comments end at LF, CRLF, CR, or EOF.

Spans are half-open byte ranges in the original source, including any preamble.
Line and column numbers start at one. Columns count Unicode scalar values, with
tabs counting as one; a leading BOM has zero column width. LF, CRLF, and CR each
represent one line break.

Names use ASCII letters or underscores, followed by letters, digits, or
underscores. Names are case-sensitive. Variables start with `$`; reading an
undefined variable raises an error. Assignment creates or replaces a variable,
and unannotated variables may change type.

Values are `null`, `bool`, signed 64-bit `int`, finite 64-bit `float`, UTF-8
`string`, separate `list` and `map` collections, host-defined native objects,
and callable values. Integer literals currently contain decimal digits only; floating-point
literals have digits on both sides of a decimal point, an exponent, or both.
Exponents use `e`/`E`, an optional sign, and at least one digit. Signs outside
exponents are operator tokens. A dot without following digits is concatenation;
`.5` is a dot and an integer, not one float literal. Hexadecimal, binary, numeric
separators, identifier suffixes, and repeated fractional parts are unsupported.
Tokens retain numeric spelling; the parser checks integer and finite float ranges
without converting between types. Unary signs are preserved as AST operators.
An integer token must fit in `i64` before unary operators are applied.

Double-quoted strings support `\n`, `\r`, `\t`, `\0`, `\\`, `\'`, `\"`, and `\$`.
Single-quoted strings support only `\\` and `\'`. Other escapes fail. Strings may
contain literal Unicode and newlines. Interpolation is not implemented; `$name`
inside either string form is literal text. Raw source spans include both quotes;
token values contain decoded escapes.

Simple statements end in `;`; braced compound statements do not require a
trailing semicolon. Assignment is an expression and may appear within another
expression. The complete statement grammar is specified below.
Echo accepts exactly one string expression, adds no newline, and never performs
implicit conversion. Parentheses group expressions, so both echo forms work.
Native calls accept comma-separated arguments; trailing commas are not supported.

In the executable subset, the `.` operator concatenates two strings. `===`
compares values without coercion:
`1 === 1.0`, `1 === "1"`, and `true === 1` are all false. Cross-type comparison is
false rather than an exception. Concatenation binds more tightly than equality;
both operators associate left to right. Parentheses override precedence.
`!==` is the negation of strict equality.

```scr
<?scr
$usage = 85;
echo "Disk usage: " . str($usage) . "%\n";
echo str(1 === 1.0); // false
```

`"Disk usage: " . $usage` is a type error. The explicit conversion in the example
above corrects the implicit conversion in the original design sketch.

## Lexer coverage and equality policy

The lexer recognizes these keywords: `true false null echo if elseif else while
for foreach as function return break continue try catch finally throw exit`. Keywords
are case-sensitive; `$if` is a variable, and `if_name` is an identifier.

Operators and punctuation: `+ - * / % . = += -= *= /= .= == != === !== < <= > >=
&& || ! ; , : ( ) { } [ ] => -> ::`. Longest supported spellings take precedence.
Unsupported operators such as `++`, `--`, `**`, `%=` and bitwise operators fail.

Only `===` and `!==` are allowed equality operators in Scripta. Legacy `==` and
`!=` are tokenized so the parser can reject them with a targeted diagnostic.
The expression parser implements both strict equality operators and rejects
legacy equality with a diagnostic pointing to the offending operator.

`scr check` validates both lexical structure and statement syntax, including the
equality policy. It reports parser diagnostics without executing the program.
`scr run` parses the complete program before attempting execution.

## Expression grammar and AST

The parser uses recursive descent for statements, argument lists, and collection
literals, with Pratt parsing for expressions. Every expression node retains a
half-open source byte span. Grouped expressions remain distinct AST nodes: their
spans include parentheses while their child spans exclude them. Calls, access,
and collection spans include their closing delimiter or final member name.
No evaluation, constant folding, or implicit type conversion occurs in parsing.

Precedence is explicitly defined below, from lowest to highest. Operators on the
same row share precedence. Binary operators associate left to right; assignments
associate right to left. Prefix operators nest from right to left, and postfix
operations chain from left to right.

| Level | Syntax | Meaning / associativity |
| --- | --- | --- |
| 1 | `= += -= *= /= .=` | Assignment, right |
| 2 | `\|\|` | Logical OR, left |
| 3 | `&&` | Logical AND, left |
| 4 | `=== !==` | Strict equality / inequality, left |
| 5 | `< <= > >=` | Comparison, left |
| 6 | `.` | String concatenation, left |
| 7 | `+ -` | Addition / subtraction, left |
| 8 | `* / %` | Multiplication / division / remainder, left |
| 9 | `+ - !` (prefix) | Unary positive / negative / logical NOT, right |
| 10 | `(...) [...] ->name ::name(...)` (postfix) | Call / index / property / native call, left |

Arithmetic and concatenation are separate operators: `"Count: " . 1 + 2`
parses as `"Count: " . (1 + 2)` without inserting a conversion. Any operand type
errors belong to evaluation. Property access always uses `->`; `$file->name`
cannot be confused with `$file . name`, which is concatenation.

Primary expressions are `null`, booleans, integer/float/string literals,
`$variable`, bare identifiers such as `str` or `Process`, parenthesized
expressions, bracketed collections, and anonymous function literals.
Identifiers represent callable/native
names and remain distinct from variables in the AST. Prefix operators accept
any expression at their binding power. A call has an expression callee and zero
or more comma-separated arguments; calls may chain, for example `$f()(1)`.
Call argument lists do not accept trailing commas.

Native calls use `Identifier::name(arguments)`, for example
`Process::run("git", ["status"])`. The namespace must be a bare identifier and
the member must be followed by an argument list. This is native API syntax;
classes and object declarations are not part of this grammar. Property members
must be identifiers; dynamic member syntax is not supported. Index reads require
one expression inside brackets. Empty brackets are a write-only list append
target and must be followed immediately by plain `=` assignment.

A list uses `[expression, ...]`. A map uses `[key => value, ...]`, preserving
entry order and each key/value expression in the AST. The first entry determines
whether a nonempty collection is a list or map; mixing the forms is an error.
`[]` is an empty list; there is no empty map literal. `Map::create()` constructs
an empty map through the standard library. Collections
accept an optional trailing comma. Keys are expressions that must evaluate to
`int` or `string`. Keys retain their types: `1` and `"1"` are different keys.
Duplicate keys replace their earlier value without changing iteration position.

Assignment targets may be variables, properties, or indexes rooted in a valid
assignment target. Grouped expressions are not assignment targets.
`$a = $b = 10` parses as `$a = ($b = 10)`; compound assignments use the same
precedence and associativity and retain their own AST operators.

```scr
<?scr
$count = 10;
$name = "Scripta";
$total = ($count + 5) * 2;
$text = "Count: " . str($count);
$items = [1, 2, 3];
$config = ["host" => "localhost", "port" => 8080];
$value = $config["host"];
$result = Process::run("git", ["status"]);
$name = $file->name;
```

These examples are accepted by the parser. Expressions execute as specified
below. The standard library registers `Process::run` for literal argv execution,
filesystem operations through `File`, `Dir`, and `Path`, and environment/CLI APIs. Properties
require a native object, such as the entries returned by `Dir::files`. `scr check` validates
syntax without resolving functions or variables or running the program.

## Statement grammar

Every statement has a source span covering its keyword or expression through its
terminating semicolon or complete body. A block is `{ statement* }`, including
an empty block, and is itself a statement. Closing `?>` is lexer trivia and never
produces a statement or substitutes for a required semicolon. Empty statements
(a standalone `;`) are not supported.

```text
statement = expression ";"
          | "echo" expression ";"
          | block
          | "if" "(" expression ")" statement
              { "elseif" "(" expression ")" statement }
              [ "else" statement ]
          | "while" "(" expression ")" statement
          | "for" "(" expressions ";" expressions ";" expressions ")" statement
          | "foreach" "(" expression "as" variable [ "=>" variable ] ")" statement
          | "break" ";" | "continue" ";"
          | "function" identifier "(" parameters ")" [ ":" type ] block
          | "return" [ expression ] ";"
          | "try" block ( catch { catch } [ "finally" block ] | "finally" block )
          | "throw" expression ";"
          | "exit" [ expression ] ";"
          | "exit" "(" [ expression ] ")" ";"
block = "{" { statement } "}"
expressions = [ expression { "," expression } ]
parameters = [ parameter { "," parameter } ]
parameter = variable [ ":" type ] [ "=" expression ]
          | "..." variable [ ":" type ]
anonymous_function = "function" "(" parameters ")" [ ":" type ] block
catch = "catch" "(" [ type ] variable ")" block
type = identifier | "null"
```

`echo` is a language statement accepting exactly one expression in both forms:
`echo "text";` and `echo("text");`. Parentheses create a grouped expression
within that same statement; echo is not an ordinary callable identifier. Both
forms use identical output behavior and type rules. Empty or multiple echo
arguments are errors. Output adds no newline and requires explicit conversions.

Control-flow bodies may be a single statement or a block. An `else` or `elseif`
attaches to the nearest unfinished `if`. Orphan `else`, `elseif`, `catch`, and `finally`
clauses are errors. Braced control-flow bodies, standalone blocks, function
bodies, and try/catch/finally blocks do not consume a trailing semicolon. A single simple
statement used as a body still needs its own semicolon.

Each `for` header contains exactly two semicolon separators. Its three sections
are optionally empty comma-separated expression lists without trailing commas;
`for (;;) { break; }` is valid. Header expressions execute left to right; the
last condition expression must be bool, and an empty condition means true.
`continue` executes the increment section before testing the condition again.
`foreach`
supports value bindings (`as $value`) and key/value bindings
(`as $key => $value`); bindings must be variables, not arbitrary expressions.

`break;` and `continue;` are only valid inside loops. Numeric loop levels and
labels are unsupported. `return;` and `return expression;` are only valid inside
functions. A function boundary resets the loop context, even when the function
is declared in a loop. Nested functions are parsed with their own context.

Functions have mandatory braced bodies and can be named declarations or
anonymous expression literals, `function (...) { ... }`. A literal registers no
function name and evaluates to a callable; each evaluation creates a distinct
callable identity. A literal used as an expression statement requires a
semicolon, while a named declaration does not. Return types use `: type` after
the closing parenthesis.

### Function parameters and captures

Parameters use `$name` or `$name: type`, optionally followed by `= expression`.
Required parameters must precede defaults. A final `...$values` parameter
collects remaining positional arguments into a new list; `...$values: int`
checks each extra argument as an int, not the resulting list. It binds an empty
list when there are no extra arguments. Only one variadic parameter is allowed,
it must be last, and it cannot have a default. Parameter and call argument lists
do not accept trailing commas. Duplicate parameter names and redeclaration of a
named function in the same scope remain errors.

Calls evaluate supplied arguments left to right in the caller before arity and
annotation checks, as before. Missing defaults evaluate left to right, once per
call, in the callee's parameter scope before its body. They see earlier bound
parameters and captured enclosing function locals; later parameters are not yet
bound and the caller's unrelated locals are not visible. A supplied argument,
including explicit null, suppresses that parameter's default. Collection default
expressions are evaluated afresh, so mutations do not persist between calls.
Normal side effects of an evaluated default remain visible if a later default
or annotation fails. Direct top-level named declarations are hoisted without
evaluating their defaults.

Calls require all required arguments and reject extra arguments unless there
is a variadic parameter. Parameter and return annotations require matching types
without conversion. Built-in type names are `null`, `bool`, `int`, `float`,
`string`, `list`, `map`, and `callable`; native object annotations match the host
type name. `void` return types permit only null. Falling off a function or
`return;` returns null. Annotations check call boundaries; they do not freeze
parameter bindings against later reassignment. Defaults are checked after
evaluation, and each variadic element is checked before binding its list.
The resulting list obeys the ordinary byte and nesting limits, including its
additional outer list slot and nesting level.

Anonymous literals capture environments by reference, like named nested
functions. Returned callbacks can share and update enclosing function locals.
The existing global isolation applies to both named and anonymous functions:
pass global variables explicitly. A recursive anonymous function can refer to
its own captured local binding inside an enclosing function; assignment at the
global scope does not grant a closure access to globals. Captured scopes retain
the existing runtime lifetime and scope-budget rules. Defaults and function
bodies retain their definition source for diagnostics and relative `require`.
Anonymous frames are labelled `<anonymous>`; arity/annotation failures remain
catchable `TypeError`. Default evaluation failures keep their original category
and source location, with the callee frame and its caller's call site.

```scr
<?scr
function total($initial: int = 0, ...$values: int): int {
    $sum = $initial;
    foreach ($values as $value) { $sum += $value; }
    return $sum;
}
function multiplier($factor: int): callable {
    return function ($number: int = 1): int { return $number * $factor; };
}
$double = multiplier(2);
echo str(total(10, 1, 2)) . "\n"; // 13
echo str($double(7)) . "\n"; // 14
```

Named arguments, arrow syntax, and call-site unpacking (`f(...$items)`) remain
unsupported. `...` is recognized for parameter declarations only. No classes or
object declarations are introduced. See the
[functions playground](../playground/functions.scr) for callbacks, counters,
and independent mutable defaults.

### Exceptions and control flow

A `try` requires a braced body and at least one `catch` or a final `finally`
block (or both). Catch clauses use
`catch (Error $e) { ... }` or `catch ($e) { ... }`. Types here are exception names,
not class declarations. `throw` requires one expression. `exit;`, `exit();`,
`exit expression;`, and `exit(expression);` are accepted as statements; exit is
not a general expression and accepts at most one expression.
`throw` propagates its value through function calls. Untyped catches match any
explicitly thrown value, typed catches match its type name, and `Error` matches
any explicitly thrown value. Catch bindings are local to their catch block.
Unmatched throws become runtime errors with the original throw location and
stack. Invalid user-function arity and parameter/return annotation violations
raise `TypeError` exceptions, as do invalid native arguments and conversions.
These match `catch (TypeError $e)`, `catch (Error $e)`,
or an untyped catch; their binding contains the error message string. Uncaught
TypeErrors preserve the original location and stack. Native functions can raise
named exceptions such as `IoError` and `PermissionError`; native property failures
raise `PropertyError`. These match their named catch, `Error`, or an untyped catch,
and bind the message string. Core operator and collection failures are also
catchable, with the following exception names:

| Name | Core failures |
| --- | --- |
| `IndexError` | Missing map keys, out-of-bounds or negative list positions, sparse list writes, and absent intermediate collection entries. |
| `ArithmeticError` | Division/remainder by zero (including float signed zero), integer arithmetic/negation overflow, and non-finite float arithmetic results. |
| `TypeError` | Incompatible arithmetic/comparison/concatenation operands, non-bool conditions or logical operands, invalid index/key types, indexing a non-collection, and appending to a non-list. |

These match their named catch, `Error`, or an untyped catch. Their bindings
contain the redacted diagnostic message prefixed with the exception name. A
`catch (string $e)` does not match these errors just because the exposed message
is a string. An unmatched typed catch preserves the original failure, source
span, and stack trace. `throw $e` explicitly throws a new string value; it does
not rethrow an exception object or preserve the original exception kind.

```scr
<?scr
$number = 10;
try {
    $number /= 0;
} catch (ArithmeticError $e) {
    echo "Keeping the previous value\n";
}
echo str($number); // 10
```

This changes earlier behavior: these core errors used to bypass all catches.
Uncaught errors still report E0401 with the original source span/trace and CLI
exit status 1; their messages now include the exception name. Operand coercion,
evaluation order, and collection bounds are unchanged. Failed operations do not
commit a destination write; earlier target/RHS side effects remain visible.

Other runtime failures, including undefined names and invalid call targets,
remain outside catches. Core runtime cancellation, instruction/scope/value/source
and call/value-depth limits, allocation failures, and output-stream failures
also bypass catches. Existing named native exceptions (including helper
`RangeError` and `ResourceLimitError`) retain their documented behavior; they are
distinct from core runtime safety limits. Exit, return, break, and continue
propagate through try/catch without triggering catches.

A `finally` block runs once after the try body and its selected catch, if any,
on normal completion, throw, return, break, continue, and exit. It has its own
block scope; try and catch locals are unavailable there, so declare shared state
in the enclosing scope. Normal cleanup preserves the pending outcome, including
a return value evaluated before cleanup. A new throw, return, loop control, or
exit from finally replaces the pending outcome. Replacement failures retain their
own source location, stack trace, and diagnostic redaction; the original failure
is not chained into the replacement diagnostic. Nested cleanup runs inside out.

Fatal runtime failures, including cancellation, budgets, undefined names and I/O
output failures, bypass finally as well as catches. Cleanup itself is subject to
the same execution limits and cancellation and can be interrupted; it receives
no additional budget. Forced process termination can prevent all cleanup.
Native file streams release on explicit close or last-reference drop, including
runtime teardown when the host has not retained aliases. See
[bounded file streams](standard-library.md#bounded-file-streams).

The parser reports unexpected tokens, missing semicolons, unmatched braces and
parentheses, and invalid control-flow contexts with source spans. An unclosed
block points to its opening brace; missing tokens point to the current token or
EOF. After a diagnostic, recovery may discard the remainder of that malformed
statement to a semicolon, an enclosing block boundary, or a following statement.
Nested braces are respected. Tokens are never discarded to turn invalid input
into success: any parser diagnostic makes validation fail. Recoverable errors
inside blocks are collected while preserving the enclosing control-flow context.
Lexing stops at its first error; parser errors may be collected in one pass.

## Runtime values, scopes, and execution

Top-level function declarations are registered before any other top-level
statements execute. They support forward calls and mutual recursion; their bodies
run only when called. All other statements execute synchronously in source order.
Conditional and nested declarations become available when executed. Calls
evaluate their callee and then arguments from left to right. Binary operands and
collection entries also evaluate left to right. Assignment resolves its receiver
and index expressions once before evaluating its right operand; compound
assignment reads the old value before the right operand and stores the result.

The global environment persists for the lifetime of a runtime. Each braced block
creates a child lexical scope. First assignment to a new variable creates it in
the current scope; assignment to an existing name updates its nearest lexical
binding. Variables declared only in a block disappear from name lookup on exit.
Unannotated variables may change type. Function calls create a parameter scope
whose parent is the declaration scope, not the caller scope. Function variable
lookup and assignment exclude the global scope: globals must be passed explicitly.
Parameters shadow outer bindings. Arguments, including nested lists/maps, are
passed by value using deep clones; callee collection mutations cannot affect callers. Named functions are stored in a separate lexical namespace;
bare names resolve callables, and `$name` always resolves a variable.

Functions capture lexical environments, so returned callable values may read and
update captured variables after the declaring function has returned. A function
or callable can be assigned to a variable and invoked with `$f(...)`. Closures
belong to their creating runtime; calling one from a different runtime is an
error. Native functions may be registered by a host, including names such as
`Native::method`, without introducing script classes or built-in OS APIs.

`while` bodies use their normal statement/block scope. `for` creates a loop scope
for newly assigned header variables. `foreach` evaluates its collection once and
iterates a value snapshot in list-index or map-insertion order. Its key/value
bindings live in a new loop scope and shadow outer bindings. Every braced loop
body creates a fresh child scope per iteration. Break exits the nearest loop;
continue advances to its next iteration. Scope cleanup occurs on normal exit,
return, break, continue, explicit throw, exit, and runtime errors.

### Strict operators and conversions

Arithmetic requires two numeric operands of the same runtime type. `int + int`
returns int; `float + float` returns float, and the same rule applies to `-`, `*`,
`/`, and `%`. Mixed int/float operations are errors: use `float($i)` or `int($f)`
explicitly. Integer division truncates toward zero; integer remainder has the
dividend's sign. All integer operations, including negation, division, and
remainder, check overflow. Division or remainder by zero (including float signed
zero) is an error. Floating operations must return finite values; overflow is an
error. Explicit `float(int)` conversion may round large integers to f64 precision.

`<`, `<=`, `>`, and `>=` accept matching numeric types or two strings. String
ordering is lexicographic in UTF-8 order. Other combinations are errors. Strict
`===` and `!==` never coerce, including int versus float. Lists compare by ordered
recursive value equality. Maps compare by key/value contents independent of
insertion order. Native objects and callables compare by identity.

Every if/elseif/while condition and the final for-condition value must be bool.
`!` requires bool. `&&` and `||` check the left bool operand first and evaluate the
right operand only when needed; an evaluated right operand must also be bool.
There is no implicit truthiness. `+` never concatenates strings, and `.` requires
two strings. Echo requires one string and writes its bytes without adding a
newline. Explicit conversions are registered as `str`, `int`, `float`, and `bool`;
invalid inputs or arities raise `TypeError`. `type` returns stable type names,
`len` counts string Unicode scalars or collection entries, and `sleep` accepts
nonnegative integer milliseconds and checks host cancellation. See
[standard library](standard-library.md) for the conversion matrix and native API.

The `String` namespace supplies pure text helpers for trimming, literal
split/join, search, replacement, Unicode scalar substrings, and Unicode case
conversion. These are registered native functions, not string methods or new
syntax; they do not add string interpolation or string indexing. Arguments are
strictly typed. Invalid substring bounds and empty split/replacement search
strings raise catchable `RangeError`; result budget failures raise
`ResourceLimitError`. See [string helpers](standard-library.md#strings) for
signatures, edge cases, Unicode behavior, and limits.

`Math::abs`, `min`, `max`, `floor`, `ceil`, and `round` preserve int/float types.
Binary min/max require matching numeric types and select the left value on ties.
Floats must be finite; rounding ties go away from zero. Ints pass through rounding
unchanged; abs of i64 minimum raises catchable `ArithmeticError`.

`String::format(template, values)` explicitly formats scalar values from a list
using `str` display. Sequential `{}` placeholders consume exactly one value each;
`{{` and `}}` escape literal braces. Replacement text is literal. Invalid brace
syntax or value counts raise `FormatError`; invalid types raise `TypeError`.
Unicode and argument values are preserved. The result is preflighted against the
logical output byte budget; failures raise `ResourceLimitError`. This is a native
helper and adds no interpolation syntax. See [Math](standard-library.md#math)
and [strings](standard-library.md#strings) for the full contracts.

### Collections and host objects

Lists and maps have owned value semantics: assignment or argument passing copies
the collection, including nested collections. Native object and callable handles
retain identity when copied. List indexes must be nonnegative ints. Reads and
compound assignments require an existing element. Plain assignment replaces an
existing element or appends at exactly the current length; skipping indexes is
an error, and no holes are created.

`$items[] = value` appends to an existing list, including an empty list. Nested
targets such as `$groups[0][] = value` or `$map["items"][] = value` work when the
intermediate collections exist. The target must already be a list; undefined
variables, null, scalars, and maps are not converted to lists. Empty brackets
cannot be read, used in compound assignment, or used as an intermediate target
(`$items[][] = value` is invalid). An append assignment evaluates to its assigned
value, like other assignments.

Target index expressions and native object receivers are evaluated once, before
the RHS. Appending uses the target list's length after evaluating the RHS, so
an RHS that appends to the same list does not have its new element overwritten.
The usual value nesting and optional logical-byte limits apply; a failed growth
check leaves the destination unchanged (target/RHS side effects are not rolled back).

```scr
<?scr
$items = [];
$items[] = "apple";
$items[len($items)] = "banana";
$items[0] = "pear";
echo Json::encode($items); // ["pear","banana"]
```

Map lookup requires an existing int/string key; assignment may add a final key.
Nested assignment never creates missing intermediate collections. Indexing
other types is an error.

The standard library's `List` and `Map` namespaces add collection helpers without
new syntax. List removal, slicing, concatenation, membership, and homogeneous
sorting preserve strict value semantics. `Map::has` and `Map::get` safely handle
absent keys, while normal indexing still requires an existing key. Map keys and
values can be projected in insertion order, and removal returns an independent
map. Invalid helper arguments and bounds raise catchable native errors; core
indexing and arithmetic use the exception names documented above. See the
[collection reference](standard-library.md#collections) for signatures and limits.

Property access uses host native objects. A host may expose readable or writable
properties; assigning a read-only property raises an error. The runtime contains
no class declarations. The standard library registers `File`, `Dir`, and `Path`
filesystem helpers plus Process, Env, and Cli APIs. Process results and spawned
handles are read-only native objects.

### Output, exit, and errors

Hosts supply a `std::io::Write` output destination. The runtime never changes
global stdout or terminates the process. Normal completion and `exit;`/`exit();`
return code 0; an exit expression must be an int from 0 through 255. Exit unwinds
all nested calls and loops immediately. The CLI returns the script code; file,
syntax, and runtime failures return 1, and command-line usage errors return 2.
`scr check` never executes script exits or output.

Runtime failures retain the failing source span and a structured stack trace.
The CLI prints the diagnostic and call-site frames to stderr, innermost first.
Output written before a runtime failure remains written. Lexing and parsing must
succeed for the whole program before any execution occurs. `RuntimeOptions`, passed
to `Runtime::with_options`, configures `recursion_limit` (default 64 active
script/native invocations, excluding main). The recursive evaluator caps it at
`MAX_RECURSION_LIMIT` (64) for stack safety; zero disables calls. Exceeding the
limit reports a runtime error and unwinds scopes.

Hosts may provide `interrupt_flag: Some(Arc<AtomicBool>)` and set it to true to
cancel execution, or set `instruction_limit: Some(u64)` for a deterministic budget.
Statements and loop back edges check both, including empty infinite loops. Budget
units are checkpoints, not CPU instructions, and reset on each execute call.
Cancellation flags remain set until cleared by the host. Interruption bypasses
script catches and reports a runtime error after unwinding calls and scopes.
Native calls must return before execution can observe cancellation. There are no
wall-clock or allocation limits.

The runtime uses safe Rust and owns its environments in an arena. Transient
scopes are reclaimed and slots reused. A tracing collector reclaims captured
environments once they are unreachable from globals, active calls, or callable
values retained by the evaluator or host. Collection runs automatically during
execution and at execution boundaries; hosts can request it through
`Runtime::collect_garbage()`. Function references use scope IDs, allowing recursive
closure/environment cycles to be reclaimed. Opaque native ownership cycles are
conservative roots and require host cleanup. Hosts control native object handles
through safe reference-counted ownership. Collection timing is not an explicit
resource-close guarantee, and freed memory need not immediately reduce RSS.

## Planned features

Interpolation, variable annotation syntax, richer exception types, and
callback-based collection transforms and pop remain future work.
List and map helpers for removal, slicing, concatenation, membership, sorting,
construction, projection, and safe lookup are implemented.
List growth through `[] =` and assignment at the current length is implemented.


## Local script loading

```scr
<?scr
require "lib/helpers.scr";
require_once "lib/config.scr";
```

`require expression;` and `require_once expression;` are statements. The expression
must evaluate to a nonempty string naming a local UTF-8 file. Every file is lexed
independently and must start with `<?scr` under the normal opening-tag rules
(including the existing BOM/shebang handling). Each file is fully parsed before
any of its statements execute. Dependencies are loaded only when their require
statement executes; syntax checking a parent does not recursively open dependencies.

Relative paths resolve against the directory of the **including source file**,
independent of `Dir::change` or the process cwd. A function defined in an included
file retains that definition file for later relative requires and diagnostics.
Absolute local paths are supported; remote URLs are rejected. For hosts without
a script filename, relative requires use the runtime working directory.

Included statements execute in source order, at the require statement, without
an extra scope. At top level, variables/functions become available in the caller's
top-level scope; inside a block or function, existing lexical scope rules apply.
Assignments update the nearest visible binding. Includes do not bypass function
isolation from global variables, and introduce no namespace or exported-symbol
mechanism. Included function declarations become available when encountered;
they are not hoisted before earlier statements in that included file. The entry
program retains its existing direct top-level function hoisting behavior.

`require` executes on each call (and can therefore encounter a duplicate function
declaration). `require_once` skips a file after successful execution by either
form, using its canonical path, so `..` and symlink aliases share identity.
Loaded state is local to a runtime and persists across execute calls. Failed
includes are not marked complete and may be retried; earlier side effects are
not rolled back. Active files are checked for circular inclusion before the
once check; cycles raise `CircularIncludeError`, including self-inclusion of
the entry script. Calls and includes share the runtime depth limit (maximum 64).

By default a required file may be at most 8 MiB; hosts/CLI can configure the shared
source byte ceiling. Missing/unresolvable dependencies raise
`ModuleNotFoundError` with the requested path; malformed dependencies raise
`ModuleSyntaxError` at their own source location. Path/type/encoding, depth, and
read errors are explicit exceptions. Module execution preserves throws and exit
codes. Included syntax is parsed as a complete script: top-level return/break/
continue cannot escape into the including function or loop. Nested failures
retain each source file and include/function call-site line/column in traces.

Reads require `filesystem.read` and use the standard library's FilesystemPolicy
when registered, with the same documented concurrent-path-replacement limitations.
Embedders can call `Runtime::execute_source(source, output)` with
`ContextOptions.script_path` to retain source text and entry identity, or continue
using AST execution. `Runtime::set_module_policy` supplies an additional host path
check. Hosts may opt into package-name resolution with
`Runtime::set_module_resolver`; the CLI configures this for installed direct
dependencies. Existing local files take precedence over package names. Package
entries still pass through the module policy and execute with the same include
semantics. Resolution never fetches remote code during execution; see
[packages](packages.md) for explicit installation, locking, and checksums.


## Developer checks and formatting

`scr check file.scr` parses supported syntax, checks declaration structure and
invalid control flow, and reports duplicate direct functions/parameters without
executing code. It is not complete static type checking; unknown variables,
function targets, custom annotations, and required-file dependencies may remain
unresolved until execution. Runtime annotation and strict-value checks are unchanged.

Legacy PHP `==`, `!=`, `<>`, `<=>`, `and`, `or`, `xor`, increment/decrement,
coalescing, and other unsupported operators produce explicit diagnostics. Use
`===`/`!==` and `&&`/`||` for supported comparisons and boolean operations.
The PHP word operators are reserved and cannot be used as bare identifiers.

`scr fmt file.scr` preserves lexical tokens, comments and string spelling while
normalizing layout. `scr fmt --check file.scr` returns 1 when formatting would
change the file and never writes it. See [developer tooling](development.md#formatter-and-diagnostic-tooling)
for diagnostic codes, exact formatter policy, exit codes, and parser resource limits.


Runtime-created values have a maximum nesting depth of 128. Optional per-value
logical-byte, live-scope, instruction, and source budgets affect resource use, not
value/coercion semantics. See [performance and safety](performance.md) for
configuration and current limitations.


## CLI source and pipeline input

`scr run - [arguments...]` reads UTF-8 source from stdin through EOF under the
same source-byte limit as disk files. Tags and language semantics are unchanged.
Diagnostics use `<stdin>`, and relative includes/packages resolve from the working
directory. Script input then observes EOF. `scr run -- -` selects a literal file.
File-backed scripts can consume piped records with bounded `Cli::readLine`,
which returns a string or null at EOF. See the [input API](standard-library.md#script-arguments-and-prompts)
for byte limits, permissions, cancellation, and failure recovery.
