use super::*;
use std::{cell::RefCell, rc::Rc};

fn run(
    runtime: &mut Runtime,
    source: &str,
    output: &mut Vec<u8>,
) -> std::result::Result<u8, RuntimeError> {
    let tokens = scr_lexer::lex(&format!("<?scr {source}")).unwrap();
    runtime.execute(&scr_parser::parse(&tokens).unwrap(), output)
}
fn execute(source: &str) -> (Runtime, String, u8) {
    let mut runtime = Runtime::new();
    let mut output = Vec::new();
    let code = run(&mut runtime, source, &mut output).unwrap();
    (runtime, String::from_utf8(output).unwrap(), code)
}
fn failure(source: &str) -> RuntimeError {
    run(&mut Runtime::new(), source, &mut Vec::new()).unwrap_err()
}
fn assert_int(runtime: &Runtime, name: &str, expected: i64) {
    assert_eq!(runtime.get_global(name), Some(&Value::Int(expected)));
}

#[test]
fn hello_world_and_identical_echo_forms() {
    let (_, output, code) =
        execute(r#"$name = "World"; echo "Hello " . $name . "\n"; echo("Hello " . $name . "\n");"#);
    assert_eq!(output, "Hello World\nHello World\n");
    assert_eq!(code, 0);
}

#[test]
fn all_scalar_expressions_and_arithmetic_operators() {
    let (runtime, _, _) = execute(
        r#"
        $null = null; $yes = true; $no = !true; $i = +10;
        $f = -1.5; $text = "é" . "!";
        $add = 2 + 3; $sub = 2 - 3; $mul = 2 * 3;
        $div = -7 / 2; $rem = -7 % 2;
        $fa = 1.5 + 2.5; $fs = 1.5 - 2.5; $fm = 1.5 * 2.0;
        $fd = 7.0 / 2.0; $fr = 7.0 % 2.0;
        $group = (2 + 3) * 4; $a = $b = 5;
        $a += 2; $a -= 1; $a *= 3; $a /= 2;
        $text .= "?";
    "#,
    );
    assert_eq!(runtime.get_global("null"), Some(&Value::Null));
    assert_eq!(runtime.get_global("yes"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("no"), Some(&Value::Bool(false)));
    for (name, expected) in [
        ("i", 10),
        ("add", 5),
        ("sub", -1),
        ("mul", 6),
        ("div", -3),
        ("rem", -1),
        ("group", 20),
        ("a", 9),
        ("b", 5),
    ] {
        assert_int(&runtime, name, expected);
    }
    for (name, expected) in [
        ("f", -1.5),
        ("fa", 4.0),
        ("fs", -1.0),
        ("fm", 3.0),
        ("fd", 3.5),
        ("fr", 1.0),
    ] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::Float(expected)),
            "{name}"
        );
    }
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("é!?".into()))
    );
}

#[test]
fn list_growth_supports_empty_nested_and_indexed_targets_with_value_semantics() {
    let (runtime, _, _) = execute(
        r#"
        $items = []; $first = ($items[] = 10); $items[1] = 20;
        $copy = $items; $items[] = 30; $items[0] = 11;
        $groups = [[], ["values" => []]];
        $groups[0][] = 7; $groups[1]["values"][] = 8;
        $groups[0][1] = 9;
        function extend($values) { $values[] = 40; return $values; }
        $extended = extend($items);
        $left = []; $right = []; $left[] = $right[] = 5;
        $snapshot = [1, 2]; foreach ($snapshot as $value) { $snapshot[] = $value; }
    "#,
    );
    assert_int(&runtime, "first", 10);
    for (name, expected) in [
        ("items", vec![11, 20, 30]),
        ("copy", vec![10, 20]),
        ("extended", vec![11, 20, 30, 40]),
        ("left", vec![5]),
        ("right", vec![5]),
        ("snapshot", vec![1, 2, 1, 2]),
    ] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::List(expected.into_iter().map(Value::Int).collect())),
            "{name}"
        );
    }
    assert_eq!(
        runtime.get_global("groups"),
        Some(&Value::List(vec![
            Value::List(vec![Value::Int(7), Value::Int(9)]),
            Value::Map(vec![(
                MapKey::String("values".into()),
                Value::List(vec![Value::Int(8)])
            )]),
        ]))
    );
}

#[test]
fn append_evaluates_targets_once_and_uses_the_list_after_rhs_side_effects() {
    let (runtime, _, _) = execute(
        r#"
        function build() {
            $items = []; $nested = [[]]; $calls = 0;
            function index() { $calls += 1; return 0; }
            function add() { $items[] = 1; return 2; }
            $nested[index()][] = 7;
            $items[] = add();
            return [$items, $nested, $calls];
        }
        $result = build();
    "#,
    );
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::List(vec![
            Value::List(vec![Value::Int(1), Value::Int(2)]),
            Value::List(vec![Value::List(vec![Value::Int(7)])]),
            Value::Int(1),
        ]))
    );
}

#[test]
fn invalid_list_growth_does_not_create_variables_or_intermediate_collections() {
    for (source, message) in [
        ("$missing[] = 1;", "undefined variable"),
        ("$items = null; $items[] = 1;", "append requires a list"),
        ("$items = 1; $items[] = 1;", "append requires a list"),
        ("$items = [0 => 1]; $items[] = 1;", "append requires a list"),
        ("$items = []; $items[-1] = 1;", "nonnegative int"),
        ("$items = []; $items[\"0\"] = 1;", "nonnegative int"),
        ("$items = []; $items[1] = 1;", "out of bounds"),
        ("$items = []; $items[0] += 1;", "out of bounds"),
        ("$items = []; $items[0][] = 1;", "out of bounds"),
        ("$items = []; $items[0][0] = 1;", "out of bounds"),
        (
            "$items = [\"existing\" => []]; $items[\"missing\"][] = 1;",
            "map key not found",
        ),
    ] {
        assert!(failure(source).message.contains(message), "{source}");
    }
    let mut runtime = Runtime::new();
    run(&mut runtime, "$items = [1];", &mut Vec::new()).unwrap();
    assert!(run(&mut runtime, "$items[] = 1 / 0;", &mut Vec::new()).is_err());
    assert_eq!(
        runtime.get_global("items"),
        Some(&Value::List(vec![Value::Int(1)]))
    );
}

#[test]
fn strict_equality_comparisons_and_short_circuiting() {
    let (runtime, _, _) = execute(
        r#"
        $a = 1 === 1; $b = 1 !== 1.0; $c = 1 === "1"; $d = null === null;
        $lt = 1 < 2; $le = 2 <= 2; $gt = 3.0 > 2.0; $ge = "b" >= "a";
        $skipped = 0;
        $and = false && ($skipped = 1);
        $or = true || $undefined;
        $yes = true && true; $no = false || false;
    "#,
    );
    for name in ["a", "b", "d", "lt", "le", "gt", "ge", "or", "yes"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(true)), "{name}");
    }
    for name in ["c", "and", "no"] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::Bool(false)),
            "{name}"
        );
    }
    assert_int(&runtime, "skipped", 0);
}

#[test]
fn scopes_update_nearest_binding_and_discard_new_block_bindings() {
    let (runtime, _, _) =
        execute("$x = 1; { $x = 2; $local = 3; { $x += 1; $local += 1; } } $x = \"changed\";");
    assert_eq!(
        runtime.get_global("x"),
        Some(&Value::String("changed".into()))
    );
    assert!(runtime.get_global("local").is_none());
    assert!(
        failure("{ $local = 1; } echo $local;")
            .message
            .contains("undefined variable")
    );
    assert!(failure("$x = 1; function read() { return $x; } function caller($x) { return read(); } caller(99);").message.contains("undefined variable $x"));
    let (runtime, _, _) =
        execute("$x = 1; function write() { $x = 99; return $x; } $result = write();");
    assert_int(&runtime, "result", 99);
    assert_int(&runtime, "x", 1);
}

#[test]
fn branching_requires_bool_and_executes_one_branch() {
    let (runtime, output, _) = execute(
        r#"$x = 0; if (false) { $x = 1; } elseif (true) { $x = 2; echo "yes"; } else { $x = 3; } if (false) echo "bad"; else echo "!";"#,
    );
    assert_int(&runtime, "x", 2);
    assert_eq!(output, "yes!");
}

#[test]
fn while_for_foreach_break_and_continue() {
    let (runtime, _, _) = execute(
        r#"
        $i = 0; $sum = 0;
        while ($i < 6) { $i += 1; if ($i === 2) continue; if ($i === 5) break; $sum += $i; }
        $total = 0;
        for ($j = 0; $j < 5; $j += 1) { if ($j === 1) continue; $total += $j; }
        $count = 0;
        for (;;) { $count += 1; if ($count === 3) break; }
        $foreach = 0; $keys = 0;
        foreach ([1, 2, 3] as $key => $value) { $foreach += $value; $keys += $key; }
        $mapped = "";
        foreach (["a" => "x", "b" => "y"] as $key => $value) { $mapped .= $key . $value; }
        $nested = 0;
        for ($a = 0; $a < 3; $a += 1) { while (true) { $nested += 1; break; } }
    "#,
    );
    for (name, expected) in [
        ("sum", 8),
        ("total", 9),
        ("count", 3),
        ("foreach", 6),
        ("keys", 3),
        ("nested", 3),
    ] {
        assert_int(&runtime, name, expected);
    }
    assert_eq!(
        runtime.get_global("mapped"),
        Some(&Value::String("axby".into()))
    );
    for name in ["j", "key", "value", "a"] {
        assert!(runtime.get_global(name).is_none(), "{name}");
    }
}

#[test]
fn for_condition_lists_and_foreach_snapshot_and_shadowing() {
    let (runtime, _, _) = execute(
        "$ticks = 0; $i = 0; for (; $ticks += 1, $i < 2; $i += 1) {} $items = [1, 2, 3]; $sum = 0; $value = 99; foreach ($items as $value) { $items[0] = 100; if ($value === 2) continue; $sum += $value; }",
    );
    assert_int(&runtime, "ticks", 3);
    assert_int(&runtime, "sum", 4);
    assert_int(&runtime, "value", 99);
}

#[test]
fn functions_returns_recursion_and_lexical_closures() {
    let (runtime, _, _) = execute(
        r#"
        function fact($n: int): int { if ($n <= 1) return 1; return $n * fact($n - 1); }
        $factorial = fact(6);
        function make($seed: int): callable {
            function next(): int { $seed += 1; return $seed; }
            return next;
        }
        $counter = make(10); $first = $counter(); $second = $counter();
        function empty(): void { return; } $none = empty();
        function implicit() {} $implicit = implicit();
        function early() { for (;;) { return 7; } } $early = early();
    "#,
    );
    for (name, expected) in [
        ("factorial", 720),
        ("first", 11),
        ("second", 12),
        ("early", 7),
    ] {
        assert_int(&runtime, name, expected);
    }
    for name in ["none", "implicit"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Null));
    }
    assert!(runtime.get_global("seed").is_none());
}

#[test]
fn call_arity_and_annotations_are_strict() {
    for (source, message) in [
        ("function f($a) {} f();", "expects 1 arguments"),
        (
            "function f($a: int) {} f(1.0);",
            "type annotation requires int",
        ),
        (
            "function f(): int { return \"x\"; } f();",
            "type annotation requires int",
        ),
        ("function f(): int {} f();", "type annotation requires int"),
        ("function f($a, $a) {}", "duplicate parameter"),
        ("function f() {} function f() {}", "already declared"),
        ("$f = 1; $f();", "call requires a callable"),
    ] {
        assert!(failure(source).message.contains(message), "{source}");
    }
}

#[test]
fn owned_collections_nested_assignment_and_map_equality() {
    let (runtime, _, _) = execute(
        r#"
        $a = [["x" => 1], [2, 3]]; $b = $a;
        $a[0]["x"] += 4; $a[1][0] = 7;
        $x = $a[0]["x"]; $copied = $b[0]["x"];
        $map = [1 => "int", "1" => "string", 1 => "updated"];
        $map["new"] = true;
        $typed = $map[1] !== $map["1"];
        $equal = ["a" => 1, "b" => [2]] === ["b" => [2], "a" => 1];
        $unequal = [1] !== [1.0]; $different = [] !== [0 => 1];
    "#,
    );
    assert_int(&runtime, "x", 5);
    assert_int(&runtime, "copied", 1);
    for name in ["typed", "equal", "unequal", "different"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(true)), "{name}");
    }
    let Value::Map(entries) = runtime.get_global("map").unwrap() else {
        panic!()
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[0],
        (MapKey::Int(1), Value::String("updated".into()))
    );
}

#[test]
fn assignment_targets_and_arguments_are_evaluated_once_left_to_right() {
    let (runtime, _, _) = execute(
        "$a = [1, 2]; $i = 0; function index($tick) { return 0; } $a[index($i += 1)] += 3; $indexed = $i; function pair($a, $b) { return [$a, $b]; } $args = pair($i = 10, $i += 1); $value = $a[0];",
    );
    assert_int(&runtime, "value", 4);
    assert_int(&runtime, "indexed", 1);
    assert_int(&runtime, "i", 11);
    assert_eq!(
        runtime.get_global("args"),
        Some(&Value::List(vec![Value::Int(10), Value::Int(11)]))
    );
    let (runtime, _, _) =
        execute("$a = [1, 2]; function change() { $a = [0, 9]; return 3; } $a[0] += change();");
    assert_eq!(
        runtime.get_global("a"),
        Some(&Value::List(vec![Value::Int(4), Value::Int(2)]))
    );
}

#[test]
fn arithmetic_overflow_zero_division_and_float_overflow_are_errors() {
    for source in [
        "9223372036854775807 + 1;",
        "(-9223372036854775807 - 1) - 1;",
        "9223372036854775807 * 2;",
        "-(-9223372036854775807 - 1);",
        "(-9223372036854775807 - 1) / -1;",
        "(-9223372036854775807 - 1) % -1;",
        "1e308 * 1e308;",
        "1e308 + 1e308;",
    ] {
        assert!(failure(source).message.contains("overflow"), "{source}");
    }
    for source in ["1 / 0;", "1 % 0;", "1.0 / 0.0;", "1.0 % -0.0;"] {
        assert!(
            failure(source).message.contains("division by zero"),
            "{source}"
        );
    }
}

#[test]
fn no_implicit_numeric_string_or_boolean_conversions() {
    for source in [
        "1 + 1.0;",
        "1 + \"2\";",
        "true + 1;",
        "null + 1;",
        "[] + [];",
        "\"1\" . 1;",
        "1 . 2;",
        "1 < 1.0;",
        "true < false;",
        "if (1) {}",
        "while (0) {}",
        "for (;1;) {}",
        "foreach (1 as $v) {}",
        "!1;",
        "+true;",
        "-\"1\";",
        "true && 1;",
        "false || 0;",
        "1 && true;",
        "echo 1;",
        "echo null;",
    ] {
        failure(source);
    }
    // A skipped RHS is not evaluated or type-checked.
    execute("true || 1; false && 1;");
}

#[test]
fn collection_access_and_mutation_errors_are_actionable() {
    for (source, message) in [
        ("$a = [1]; $a[-1];", "nonnegative int"),
        ("$a = [1]; $a[1];", "out of bounds"),
        ("$a = [1]; $a[\"0\"];", "nonnegative int"),
        ("$a = [1]; $a[2] = 2;", "out of bounds"),
        ("$a = [\"x\" => 1]; $a[\"y\"];", "key not found"),
        ("[true => 1];", "map keys"),
        ("$a = 1; $a[0];", "list or map"),
        ("$a[0] = 1;", "undefined variable"),
        (
            "$a = [\"x\" => 1]; $a[\"missing\"][0] = 1;",
            "key not found",
        ),
        ("$a = 1; $a->name;", "native object"),
        ("$a = 1; $a->name = 2;", "native object"),
        ("$undefined += 1;", "undefined variable"),
    ] {
        assert!(failure(source).message.contains(message), "{source}");
    }
}

#[test]
fn exit_returns_codes_and_unwinds_calls_without_stopping_the_host() {
    let (_, output, code) =
        execute(r#"function stop() { echo "before"; exit(7); } $x = stop(); echo "after";"#);
    assert_eq!(code, 7);
    assert_eq!(output, "before");
    assert_eq!(execute("exit;").2, 0);
    assert_eq!(execute("exit();").2, 0);
    assert_eq!(execute("exit 255;").2, 255);
    for source in ["exit -1;", "exit 256;", "exit true;", "exit \"1\";"] {
        assert!(failure(source).message.contains("exit code"));
    }
}

#[test]
fn explicit_throws_catch_by_type_and_propagate_other_control_flow() {
    let (runtime, output, code) = execute(
        r#"
        $caught = "";
        function raise() { throw "problem"; }
        try { raise(); } catch (int $e) { echo "bad"; } catch (string $e) { $caught = $e; }
        try { throw null; } catch (null $e) { echo "null"; }
        function f() { try { return 9; } catch ($e) { return 0; } }
        $returned = f();
        while (true) { try { break; } catch ($e) { echo "bad"; } }
        try { exit(3); } catch ($e) { echo "bad"; }
    "#,
    );
    assert_eq!(
        runtime.get_global("caught"),
        Some(&Value::String("problem".into()))
    );
    assert!(runtime.get_global("e").is_none());
    assert_int(&runtime, "returned", 9);
    assert_eq!(output, "null");
    assert_eq!(code, 3);
    assert!(
        failure("try { throw 1; } catch (string $e) {}")
            .message
            .contains("uncaught")
    );
    assert!(
        failure("try { 1 / 0; } catch ($e) {}")
            .message
            .contains("division by zero")
    );
    execute(
        "try { throw 1; } catch (Error $e) {} try { try { throw true; } catch (int $e) {} } catch ($e) {}",
    );
}

#[test]
fn runtime_errors_have_precise_spans_and_inner_first_stack_traces() {
    let source =
        "<?scr function inner() { return 1 / 0; } function outer() { return inner(); } outer();";
    let tokens = scr_lexer::lex(source).unwrap();
    let error = Runtime::new()
        .execute(&scr_parser::parse(&tokens).unwrap(), &mut Vec::new())
        .unwrap_err();
    assert_eq!(&source[error.span.start..error.span.end], "1 / 0");
    assert_eq!(
        error
            .trace
            .iter()
            .map(|frame| frame.function.as_str())
            .collect::<Vec<_>>(),
        ["<main>", "outer", "inner"]
    );
    let rendered = error.render("trace.scr", source);
    assert!(rendered.find("at inner").unwrap() < rendered.find("at outer").unwrap());
    assert!(rendered.contains("trace.scr:1:"));
    let error = failure("function f() { throw 1; } f();");
    assert_eq!(error.trace.last().unwrap().function, "f");
}

#[test]
fn scope_and_stack_cleanup_after_errors_and_exit_allows_runtime_reuse() {
    let mut runtime = Runtime::new();
    let error = run(
        &mut runtime,
        "function fail() { $local = 1; return $missing; } fail();",
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(error.trace.len(), 2);
    run(&mut runtime, "$global = 1;", &mut Vec::new()).unwrap();
    assert_int(&runtime, "global", 1);
    assert!(runtime.get_global("local").is_none());
    assert!(runtime.stack.is_empty());
    assert_eq!(runtime.environments.current, 0);
    run(
        &mut runtime,
        "function stop() { { exit(9); } } stop();",
        &mut Vec::new(),
    )
    .unwrap();
    run(&mut runtime, "$next = 2;", &mut Vec::new()).unwrap();
    assert_int(&runtime, "next", 2);
}

#[test]
fn bounded_recursion_reports_an_error_instead_of_recursing_forever() {
    let error = failure("function recurse() { return recurse(); } recurse();");
    assert!(error.message.contains("maximum call depth"));
    assert!(error.trace.len() <= RuntimeOptions::default().recursion_limit + 1);
}

#[derive(Debug)]
struct TestObject {
    name: RefCell<Value>,
}
impl NativeObject for TestObject {
    fn type_name(&self) -> &str {
        "TestObject"
    }
    fn get_property(&self, name: &str) -> std::result::Result<Value, String> {
        if name == "name" {
            Ok(self.name.borrow().clone())
        } else {
            Err(format!("unknown property {name}"))
        }
    }
    fn set_property(&self, name: &str, value: Value) -> std::result::Result<(), String> {
        if name == "name" {
            *self.name.borrow_mut() = value;
            Ok(())
        } else {
            Err(format!("unknown property {name}"))
        }
    }
}

#[test]
fn native_calls_static_calls_properties_and_identity_values() {
    fn double(args: &[Value]) -> std::result::Result<Value, String> {
        let [Value::Int(value)] = args else {
            return Err("expected one int".into());
        };
        value
            .checked_mul(2)
            .map(Value::Int)
            .ok_or("overflow".into())
    }
    let mut runtime = Runtime::new();
    runtime
        .register(
            "Native::double",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                |_, args| double(args).map_err(NativeError::from),
            ),
        )
        .unwrap();
    let object = Rc::new(TestObject {
        name: RefCell::new(Value::String("original".into())),
    });
    runtime
        .set_global("object", Value::NativeObject(object.clone()))
        .unwrap();
    run(&mut runtime, r#"$d = Native::double(21); $object->name .= "!"; $name = $object->name; $same = $object === $object; function f() {} $f = f; $equal = $f === f;"#, &mut Vec::new()).unwrap();
    assert_int(&runtime, "d", 42);
    assert_eq!(
        runtime.get_global("name"),
        Some(&Value::String("original!".into()))
    );
    assert_eq!(*object.name.borrow(), Value::String("original!".into()));
    assert_eq!(runtime.get_global("same"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("equal"), Some(&Value::Bool(true)));
    let error = run(&mut runtime, "Native::double(true);", &mut Vec::new()).unwrap_err();
    assert_eq!(error.trace.last().unwrap().function, "Native::double");
    run(
        &mut runtime,
        "$object->name = [1, 2]; $object->name[0] = 7; $prop = $object->name[0];",
        &mut Vec::new(),
    )
    .unwrap();
    assert_int(&runtime, "prop", 7);
    run(
        &mut runtime,
        "$object->name[] = 8; $object->name[3] = 9;",
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        *object.name.borrow(),
        Value::List(vec![
            Value::Int(7),
            Value::Int(2),
            Value::Int(8),
            Value::Int(9)
        ])
    );
}

#[test]
fn host_values_are_validated_and_foreign_closures_are_rejected() {
    fn invalid(_: &[Value]) -> std::result::Result<Value, String> {
        Ok(Value::Float(f64::INFINITY))
    }
    let mut runtime = Runtime::new();
    assert!(
        runtime
            .set_global("invalid", Value::Float(f64::NAN))
            .is_err()
    );
    assert!(
        runtime
            .set_global("invalid", Value::List(vec![Value::Float(f64::INFINITY)]))
            .is_err()
    );
    assert!(
        runtime
            .set_global(
                "invalid",
                Value::Map(vec![
                    (MapKey::Int(1), Value::Null),
                    (MapKey::Int(1), Value::Null)
                ])
            )
            .is_err()
    );
    runtime
        .register(
            "invalid",
            NativeFunction::new(NativeSignature::new(vec![], NativeType::Any), |_, args| {
                invalid(args).map_err(NativeError::from)
            }),
        )
        .unwrap();
    assert!(
        run(&mut runtime, "invalid();", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("finite")
    );
    run(&mut runtime, "function f() {} $f = f;", &mut Vec::new()).unwrap();
    let mut other = Runtime::new();
    other
        .set_global("f", runtime.get_global("f").unwrap().clone())
        .unwrap();
    assert!(
        run(&mut other, "$f();", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("different runtime")
    );
}

#[test]
fn output_is_captured_and_io_failures_preserve_script_frames() {
    struct Fail;
    impl Write for Fail {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let tokens = scr_lexer::lex("<?scr function output() { echo \"hello\"; } output();").unwrap();
    let error = Runtime::new()
        .execute(&scr_parser::parse(&tokens).unwrap(), &mut Fail)
        .unwrap_err();
    assert!(error.message.contains("output failed: closed"));
    assert_eq!(error.trace.last().unwrap().function, "output");
    let mut bytes = Vec::new();
    assert!(
        run(
            &mut Runtime::new(),
            "echo \"before\"; 1 / 0; echo \"after\";",
            &mut bytes
        )
        .is_err()
    );
    assert_eq!(bytes, b"before");
}

#[test]
fn captured_environments_release_owned_values_when_runtime_is_dropped() {
    let object = Rc::new(TestObject {
        name: RefCell::new(Value::String("owned".into())),
    });
    {
        let mut runtime = Runtime::new();
        runtime
            .set_global("object", Value::NativeObject(object.clone()))
            .unwrap();
        run(&mut runtime, "function make($held) { function read() { return $held; } return read; } $closure = make($object); $object = null;", &mut Vec::new()).unwrap();
        assert!(Rc::strong_count(&object) > 1);
    }
    assert_eq!(Rc::strong_count(&object), 1);
}

#[test]
fn read_only_host_properties_fail_and_uncaught_string_throws_retain_the_message() {
    #[derive(Debug)]
    struct ReadOnly;
    impl NativeObject for ReadOnly {
        fn type_name(&self) -> &str {
            "ReadOnly"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Ok(Value::Null)
        }
    }
    let mut runtime = Runtime::new();
    runtime
        .set_global("object", Value::NativeObject(Rc::new(ReadOnly)))
        .unwrap();
    let error = run(&mut runtime, "$object->name = \"new\";", &mut Vec::new()).unwrap_err();
    assert!(error.message.contains("read-only"));
    assert!(
        failure("throw \"useful message\";")
            .message
            .contains("useful message")
    );
}
