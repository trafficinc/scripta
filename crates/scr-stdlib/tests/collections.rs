use scr_runtime::{MapKey, NativeObject, Runtime, RuntimeOptions, Value};
use std::rc::Rc;

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}

fn execute(runtime: &mut Runtime, source: &str) {
    runtime
        .execute_source(&format!("<?scr {source}"), &mut Vec::new())
        .unwrap();
}

fn evaluate(expression: &str) -> Value {
    let mut runtime = runtime(None);
    execute(&mut runtime, &format!("$result = {expression};"));
    runtime.get_global("result").unwrap().clone()
}

fn ints(values: &[i64]) -> Value {
    Value::List(values.iter().copied().map(Value::Int).collect())
}

#[test]
fn removal_slice_and_concat_return_contiguous_lists() {
    for (expression, expected) in [
        ("List::remove([1, 2, 3], 0)", vec![2, 3]),
        ("List::remove([1, 2, 3], 1)", vec![1, 3]),
        ("List::remove([1, 2, 3], 2)", vec![1, 2]),
        ("List::remove([1], 0)", vec![]),
        ("List::slice([1, 2, 3], 1)", vec![2, 3]),
        ("List::slice([1, 2, 3], 0, 3)", vec![1, 2, 3]),
        ("List::slice([1, 2, 3], 1, 1)", vec![2]),
        ("List::slice([1, 2, 3], 3)", vec![]),
        ("List::slice([1, 2, 3], 3, 0)", vec![]),
        ("List::slice([], 0)", vec![]),
        ("List::concat([1, 2], [3])", vec![1, 2, 3]),
        ("List::concat([], [1])", vec![1]),
        ("List::concat([1], [])", vec![1]),
        ("List::concat([], [])", vec![]),
    ] {
        assert_eq!(evaluate(expression), ints(&expected), "{expression}");
    }
}

#[test]
fn sorting_is_strict_and_preserves_equal_float_order() {
    for (expression, expected) in [
        ("List::sort([3, -1, 2, 2])", ints(&[-1, 2, 2, 3])),
        ("List::sort([3, -1, 2], true)", ints(&[3, 2, -1])),
        ("List::sort([])", ints(&[])),
        ("List::sort([1], false)", ints(&[1])),
        (
            "List::sort([2.5, -1.0, 0.0])",
            Value::List(vec![
                Value::Float(-1.0),
                Value::Float(0.0),
                Value::Float(2.5),
            ]),
        ),
        (
            r#"List::sort(["é", "a", "Z", "東京"] )"#,
            Value::List(
                ["Z", "a", "é", "東京"]
                    .map(|s| Value::String(s.into()))
                    .to_vec(),
            ),
        ),
    ] {
        assert_eq!(evaluate(expression), expected, "{expression}");
    }
    for direction in ["false", "true"] {
        let Value::List(values) = evaluate(&format!("List::sort([-0.0, 0.0, -0.0], {direction})"))
        else {
            panic!()
        };
        let bits: Vec<u64> = values
            .iter()
            .map(|v| match v {
                Value::Float(v) => v.to_bits(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            bits,
            vec![(-0.0f64).to_bits(), 0.0f64.to_bits(), (-0.0f64).to_bits()]
        );
    }
}

#[test]
fn membership_uses_strict_recursive_equality_and_callable_identity() {
    for (expression, expected) in [
        ("List::contains([], null)", false),
        ("List::contains([1], 1)", true),
        ("List::contains([1], 1.0)", false),
        ("List::contains([1], true)", false),
        (r#"List::contains([1], "1")"#, false),
        ("List::contains([null], null)", true),
        ("List::contains([[1, 2]], [1, 2])", true),
        ("List::contains([[1, 2]], [2, 1])", false),
        (
            r#"List::contains([["a" => 1, "b" => 2]], ["b" => 2, "a" => 1])"#,
            true,
        ),
        (r#"List::contains([[1 => "x"]], ["1" => "x"])"#, false),
    ] {
        assert_eq!(evaluate(expression), Value::Bool(expected), "{expression}");
    }
    let mut runtime = runtime(None);
    execute(
        &mut runtime,
        "function first() {} function second() {} $yes = List::contains([first], first); $no = List::contains([first], second);",
    );
    assert_eq!(runtime.get_global("yes"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("no"), Some(&Value::Bool(false)));
}

#[test]
fn map_creation_lookup_and_removal_preserve_key_types_and_insertion_order() {
    assert_eq!(evaluate("Map::create()"), Value::Map(vec![]));
    let mut runtime = runtime(None);
    execute(
        &mut runtime,
        r#"
        $map = Map::create(); $map[1] = "int"; $map["1"] = "text";
        $map[""] = null; $map[-1] = "negative"; $map[1] = "updated";
        $keys = Map::keys($map); $values = Map::values($map);
        $int = Map::get($map, 1); $text = Map::get($map, "1");
        $null = Map::get($map, "", "fallback");
        $missing = Map::get($map, "missing");
        $fallback = Map::get($map, "missing", 42);
        $hasNull = Map::has($map, ""); $absent = Map::has($map, 2);
        $removed = Map::remove($map, 1); $unchanged = Map::remove($map, "missing");
    "#,
    );
    assert_eq!(
        runtime.get_global("keys"),
        Some(&Value::List(vec![
            Value::Int(1),
            Value::String("1".into()),
            Value::String("".into()),
            Value::Int(-1)
        ]))
    );
    assert_eq!(
        runtime.get_global("values"),
        Some(&Value::List(vec![
            Value::String("updated".into()),
            Value::String("text".into()),
            Value::Null,
            Value::String("negative".into())
        ]))
    );
    assert_eq!(
        runtime.get_global("int"),
        Some(&Value::String("updated".into()))
    );
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("text".into()))
    );
    assert_eq!(runtime.get_global("null"), Some(&Value::Null));
    assert_eq!(runtime.get_global("missing"), Some(&Value::Null));
    assert_eq!(runtime.get_global("fallback"), Some(&Value::Int(42)));
    assert_eq!(runtime.get_global("hasNull"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("absent"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("map"), runtime.get_global("unchanged"));
    let Value::Map(removed) = runtime.get_global("removed").unwrap() else {
        panic!()
    };
    assert_eq!(
        removed.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        vec![
            MapKey::String("1".into()),
            MapKey::String("".into()),
            MapKey::Int(-1)
        ]
    );
    for expression in ["Map::keys(Map::create())", "Map::values(Map::create())"] {
        assert_eq!(evaluate(expression), ints(&[]));
    }
    assert_eq!(
        evaluate(r#"Map::remove(Map::create(), "missing")"#),
        Value::Map(vec![])
    );
}

#[test]
fn helper_results_deep_copy_collections_and_defaults_are_eager() {
    let mut runtime = runtime(None);
    execute(
        &mut runtime,
        r#"
        $original = [[1], [2]];
        $removed = List::remove($original, 1); $removed[0][] = 3;
        $slice = List::slice($original, 0); $slice[0][] = 4;
        $concat = List::concat($original, []); $concat[0][] = 5;
        $map = ["nested" => [[1]]];
        $got = Map::get($map, "nested"); $got[0][] = 6;
        $values = Map::values($map); $values[0][0][] = 7;
        $copy = Map::remove($map, "missing"); $copy["nested"][0][] = 8;
        $default = [[9]]; $fallback = Map::get($map, "missing", $default); $fallback[0][] = 10;
        function eager() {
            $calls = 0;
            function fallback() { $calls += 1; return "unused"; }
            $result = Map::get(["key" => null], "key", fallback());
            return [$calls, $result];
        }
        $eager = eager();
    "#,
    );
    assert_eq!(
        runtime.get_global("original"),
        Some(&Value::List(vec![ints(&[1]), ints(&[2])]))
    );
    assert_eq!(
        runtime.get_global("map"),
        Some(&Value::Map(vec![(
            MapKey::String("nested".into()),
            Value::List(vec![ints(&[1])])
        )]))
    );
    assert_eq!(
        runtime.get_global("default"),
        Some(&Value::List(vec![ints(&[9])]))
    );
    assert_eq!(
        runtime.get_global("eager"),
        Some(&Value::List(vec![Value::Int(1), Value::Null]))
    );
}

#[derive(Debug)]
struct Object;
impl NativeObject for Object {
    fn type_name(&self) -> &str {
        "TestObject"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no properties".into())
    }
}

#[test]
fn native_object_handles_retain_identity_through_collection_helpers() {
    let mut runtime = runtime(None);
    let object = Value::NativeObject(Rc::new(Object));
    runtime.set_global("object", object.clone()).unwrap();
    runtime
        .set_global("other", Value::NativeObject(Rc::new(Object)))
        .unwrap();
    execute(
        &mut runtime,
        "$copied = List::slice([$object], 0); $yes = List::contains($copied, $object); $no = List::contains($copied, $other); $got = Map::get([1 => $object], 1);",
    );
    assert_eq!(runtime.get_global("yes"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("no"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("got"), Some(&object));
}

#[test]
fn invalid_arguments_and_sort_types_raise_catchable_type_errors() {
    let cases = [
        "List::remove([], '0')",
        "List::remove([], 0.0)",
        "List::remove([])",
        "List::remove([], 0, 1)",
        "List::slice([], 0, null)",
        "List::slice([], 0, 0, 0)",
        "List::slice([], true)",
        "List::concat([], Map::create())",
        "List::concat([])",
        "List::concat([], [], [])",
        "List::contains(Map::create(), 1)",
        "List::contains([])",
        "List::contains([], 1, 2)",
        "List::sort([1, 1.0])",
        "List::sort([true])",
        "List::sort([null])",
        "List::sort([[]])",
        "List::sort([Map::create()])",
        "List::sort([len])",
        "List::sort([], 1)",
        "List::sort([], false, true)",
        "Map::create([])",
        "Map::has([], 'key')",
        "Map::has(Map::create(), true)",
        "Map::has(Map::create(), 1.0)",
        "Map::has(Map::create())",
        "Map::has(Map::create(), 1, 2)",
        "Map::get(Map::create(), null)",
        "Map::get(Map::create())",
        "Map::get(Map::create(), 1, 2, 3)",
        "Map::keys([])",
        "Map::keys()",
        "Map::keys(Map::create(), 1)",
        "Map::values([])",
        "Map::values()",
        "Map::values(Map::create(), 1)",
        "Map::remove(Map::create(), [])",
        "Map::remove(Map::create())",
        "Map::remove(Map::create(), 1, 2)",
    ];
    let mut runtime = runtime(None);
    for expression in cases {
        execute(
            &mut runtime,
            &format!(
                "$caught = false; try {{ {expression}; }} catch (TypeError $e) {{ $caught = true; }}"
            ),
        );
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
    }
}

#[test]
fn invalid_positions_raise_range_errors_without_changing_inputs() {
    let cases = [
        "List::remove($items, -1)",
        "List::remove($items, 3)",
        "List::remove([], 0)",
        "List::slice($items, -1)",
        "List::slice($items, 4)",
        "List::slice($items, 0, -1)",
        "List::slice($items, 2, 2)",
        "List::slice($items, 3, 1)",
        "List::slice($items, 9223372036854775807)",
        "List::slice($items, 1, 9223372036854775807)",
    ];
    let mut runtime = runtime(None);
    execute(&mut runtime, "$items = [1, 2, 3];");
    for expression in cases {
        execute(
            &mut runtime,
            &format!(
                "$caught = false; try {{ {expression}; }} catch (RangeError $e) {{ $caught = true; }}"
            ),
        );
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
        assert_eq!(runtime.get_global("items"), Some(&ints(&[1, 2, 3])));
    }
}

#[test]
fn concatenation_checks_output_budget_and_leaves_failed_assignment_unchanged() {
    let mut runtime = runtime(Some(4 * std::mem::size_of::<Value>()));
    execute(
        &mut runtime,
        "$left = [1, 2]; $right = [3, 4]; $result = 'unchanged'; $caught = false; try { $result = List::concat($left, $right); } catch (ResourceLimitError $e) { $caught = true; }",
    );
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
    execute(&mut runtime, "$result = List::slice($left, 1);");
    assert_eq!(runtime.get_global("result"), Some(&ints(&[2])));
}
