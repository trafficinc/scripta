# Objects and classes

Scripta classes provide declared fields, constructors, and instance methods.
Instances have reference identity and use the existing `->` member syntax.
String concatenation remains `.` and module access remains `::`. For collection
syntax, see [array expressions](arrays.md).

## Classes and methods

```scripta
class User {
    $name: string;
    $tags = [];

    function __construct($name: string) {
        $this->name = $name;
    }

    function greet(): string {
        return "Hello, " . $this->name;
    }
}

$user = new User("Ada");
echo $user->greet();
$user->tags[] = "developer";
$alias = $user;
$alias->name = "Grace";
echo $user->greet();
```

Class declarations belong at file top level. The entry script hoists interfaces before direct
class declarations and functions; required files retain declaration-order
execution. Fields and methods are public by default, with no export declaration.
Member names must be distinct, including between fields and methods. Built-in
type names and built-in exception names cannot be class names. General class
inheritance, visibility modifiers, static
members, traits, magic hooks, and operator overloading are not included.

Declare fields explicitly with optional type annotations and default expressions.
Defaults evaluate independently for every instance in the class declaration's
lexical environment. They cannot access an instance `$this`. Untyped fields
without defaults start as null; typed fields without defaults remain uninitialized
until assigned. Reading one raises `PropertyError`; accessing or assigning unknown
fields does too. Field annotations use the same strict types as function
parameters. A failed type/budget check preserves the old field value.

`new User(arguments)` calls `__construct` when present. Without a constructor,
only zero arguments are accepted. Constructors must return null (including
implicit return or `void`); other returns fail. Initialization failures do not
publish an instance into the assignment destination, but prior external effects
are not rolled back. Methods use existing default/variadic parameters and return
annotations. `$this` is the receiver and cannot be reassigned or used as a
parameter/loop/catch binding. Method-created closures can capture it. Reading a
method as `$user->greet` returns a callable bound to that instance.

Instances have reference identity: `$other = $user` shares the same instance,
and `===` compares instance identity. Lists and maps still copy their own storage;
objects inside copies remain shared. Native object behavior is unchanged. JSON
does not automatically serialize instances: explicitly project fields into maps
or lists. No built-in list methods are added; existing helpers and `$items[]`
remain available.

## Interfaces and composition

An interface declares public method signatures, ending in semicolons instead of
bodies. A class explicitly declares `implements Interface` or
`implements First, Second`. Compose objects by storing a dependency in a typed
field and calling its methods:

```scripta
interface Notifier {
    function send($message: string): string;
}

class EmailNotifier implements Notifier {
    function send($text: string): string {
        return "Email: " . $text;
    }
}

class UserService {
    $notifier: Notifier;

    function __construct($notifier: Notifier) {
        $this->notifier = $notifier;
    }

    function register($name: string): string {
        return $this->notifier->send("Welcome, " . $name);
    }
}

$users = new UserService(new EmailNotifier());
echo $users->register("Ada");
```

Interfaces work in field, parameter, and return annotations. Matching methods
alone do not establish conformance: `implements` is required. Instances retain
their concrete class identity and methods; interfaces provide no implementation
and cannot be instantiated. Interface and class names share a declaration
namespace. Both reject built-in type names. Interface declarations belong at file
top level and may contain only unique method signatures, without fields,
constructors, static members, or inherited interfaces.

Implementation signatures must have the same parameter count, parameter and
return types, optional positions, and variadic positions. An omitted annotation
matches only an omitted annotation. Parameter names may differ. Types resolve in
each declaration's lexical environment, so aliases referring to the same class or
interface match; unrelated same-named module types do not. This first version has
no parameter or return type variance. Defaults in interface signatures mark
optional arguments and are never evaluated; calls use the implementing method's
default expressions, whose values need not match the interface's defaults.
Multiple interfaces may require the same method when their signatures agree.
Missing or incompatible methods and unresolved implemented interfaces raise
`ClassError`; duplicate implemented interfaces are declaration errors.

Entry-script interfaces and classes are hoisted, but implementation checks run
when execution reaches a class declaration, or before its first construction if
used earlier. This allows `require "contracts" as Contracts;` followed by
`class EmailNotifier implements Contracts::Notifier { ... }`. Required files keep
declaration-order execution: declare or require interfaces before implementing
classes. `scripta check` checks declaration syntax and local duplicates; it does
not resolve interface contracts or execute signature checks.

See the [runnable composition example](../examples/interfaces.scripta) for two
interchangeable notifiers.

## Modules, types, and lifetime

```scripta
require "models" as Models;
$user = new Models::User("Ada");

function greet($user: Models::User): string {
    return $user->greet();
}
```

Module-scoped classes and interfaces are available directly through their require alias, without
exports. Methods retain the defining module's state, aliases, source locations,
and relative require resolution. Custom class annotations resolve lexical class
identity, not merely a matching name: two modules' `User` classes remain distinct.
Class-qualified field, parameter, and return annotations use class identity;
catch clauses retain the existing exception-name/type-name matching rules.
Functions and classes use separate declaration namespaces. A visible class
cannot be redeclared in the same scope. Nested direct class declarations are not
supported. Objects and bound methods belong to their creating runtime and cannot
be used to access another runtime's scope arena.

Instance fields live in tracing-managed scopes, so object self-cycles, cycles
through collections, and bound-method/closure cycles can be reclaimed. Host-held
objects and methods retain their referenced scopes while the runtime is alive.
Runtime drop releases arena-owned fields even if an opaque object handle remains
outside it. Instances share the runtime's scope and construction/call depth limits.

## Resource limits and failure behavior

Each instance's field budget includes declared field slots, field-name bytes,
and owned payload. References to other script instances occupy handles rather
than recursively expanding cyclic graphs. These are logical value and scope
limits, not a process heap cap. Failed expressions preserve assignment
destinations; script cleanup remains subject to the existing fatal-limit rules.

See [the runnable example](../examples/objects-arrays.scripta),
[native tests](testing.md), and [the language specification](language-spec.md).

## Exception classes

`class ValidationError extends Error {}` declares an exception class. A single
base may be a built-in exception, another script exception, or a module-qualified
exception such as `Errors::ValidationError`. `extends` precedes `implements`.
Ordinary classes and interfaces cannot be bases. Cycles are rejected, and an
exception hierarchy contains at most 64 classes, including `Error`.
Resolution occurs at declaration execution or first construction, like interface
validation. Required files retain declaration order.

Exception subclasses inherit fields, methods, constructors, and explicit
interface contracts. Field defaults and annotations, inherited methods, and
constructors resolve names in their declaring class's lexical environment.
Fields cannot redeclare inherited members. Methods may override inherited
methods, but must continue satisfying inherited interface contracts. There is no
`super` or parent-call syntax. Exception metadata properties cannot be redeclared
or assigned, including through compound or indexed assignment.

Every exception constructor takes a string message and optional Error/null
cause. The runtime initializes read-only metadata before invoking an inherited
or overridden script `__construct` with the supplied arguments. An overridden
constructor must therefore accept the arguments used at construction and return
null. Additional declared fields can be initialized there or through defaults.
Base-class annotations accept descendants; identity remains lexical across
module aliases. Causes preserve the original object, including custom fields
and methods. Cause chains allow at most 128 links and are traced with ordinary
script-object fields, so unreachable cycles through user fields can be reclaimed.

Metadata, retained trace strings, cause handles, custom fields, and materialized
trace collections obey logical value budgets. Creating or catching an error can
itself exceed a core budget; that failure remains fatal. Trace length shares the
runtime call-depth ceiling. Cancellation and fatal failures retain their existing
catch/finally boundaries. See [exception semantics](language-spec.md#exceptions-and-control-flow)
and the runnable [exceptions example](../examples/exceptions.scripta).
