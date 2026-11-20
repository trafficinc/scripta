# Array expressions

Scripta keeps lists and maps as distinct, strictly typed values. Array syntax
works directly with those existing values and introduces no third array type.
Lists and maps retain their value-copy semantics; object handles within them
retain reference identity. See [objects and classes](objects.md) for instance
behavior.

## Indexing, slices, and membership

```scripta
$items = [0, 1, 2, 3, 4];
$last = $items[-1];       // 4
$items[-2] = 30;
$middle = $items[1:4];    // [1, 2, 30]
$prefix = $items[:2];    // [0, 1]
$tail = $items[-2:];     // [30, 4]
$step = $items[::2];     // [0, 2, 4]
$reverse = $items[::-1]; // [4, 30, 2, 1, 0]
$found = 30 in $items;
```

Negative list indexes count from the end for both reads and writes. Positions
outside the existing list raise `IndexError`; negative writes never append.
Plain nonnegative assignment at exactly the list length still appends, and
`$items[] = value` remains supported. Negative integer map keys retain their
ordinary typed-key meaning.

Slices are read-only list expressions: `[start:stop:step]`. Bounds and step must
be ints. The stop position is excluded, omitted bounds depend on direction, and
out-of-range bounds are clamped. A zero step raises `RangeError`. Negative steps
reverse traversal. With negative steps, omitted stop means before the first
element, while an explicit `-1` means the last element; `$items[:-1:-1]` is empty.
Empty inputs and empty ranges produce empty lists. String slicing and slice
assignment are not supported. Existing `List::slice` retains its documented
nonnegative, checked-bound helper semantics.

`needle in list` uses strict recursive equality. `key in map` checks exact typed
keys, including keys whose value is null. `substring in string` requires two
strings. Other combinations raise `TypeError`. Membership has comparison
precedence; use `!($needle in $items)` for negation. No implicit truthiness or
numeric coercion is introduced.

## Comprehensions, spread, and destructuring

```scripta
$squares = [$x * $x for $x in $numbers if $x > 0];
$combined = [0, ...$left, ...$right, 99];
$result = calculate(...$arguments);
[$first, $second] = $pair;
[$head, ...$tail] = $items;
```

Comprehensions accept one `for $binding in expression` clause and an optional
`if condition`. Inputs are lists or maps; maps iterate values in insertion order,
matching `foreach`. The iterable is evaluated once. Each iteration gets a lexical
scope; the binding does not leak or overwrite an outer binding. Closures capture
their own iteration's binding. Conditions require bool. Nested comprehension
expressions are allowed; multiple clauses and map-comprehension syntax are not.

Spread accepts lists only, in list literals or call arguments. Values are
evaluated left to right, expanded in order, and copied using existing value
semantics. Objects within those collections retain identity. There is no map
spread, implicit conversion, or reference aliasing of list storage. Call-site
unpacking works with existing required/default/variadic parameters.

Destructuring uses distinct variable targets, with at most one trailing rest
variable. Only plain `=` is supported; property/index/nested patterns are not.
Without rest, lengths must match exactly; with rest, there must be at least as
many input elements as fixed bindings. Mismatches raise `RangeError`, and a
non-list input raises `TypeError`. The RHS is evaluated once, and every output is
validated before bindings are committed. Existing destinations remain unchanged
on failure; earlier RHS side effects are not rolled back. The expression's value
is the original RHS list. `$this` cannot be a binding target.

## Resource limits and failure behavior

Growing list results use fallible reservations, account for value slots, nested
collection slots, bytes, and object payload before publication, and check
cancellation. Outputs remain subject to the runtime's logical value-byte and
nesting limits. Comprehension scopes count toward the live-scope limit. These
limits are not a process heap cap. Failed expressions preserve assignment
destinations; earlier side effects are not rolled back, and script cleanup
remains subject to the existing fatal-limit rules.

See [the runnable example](../examples/objects-arrays.scripta),
[native tests](testing.md), and [the language specification](language-spec.md).
