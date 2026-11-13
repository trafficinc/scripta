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

fn assert_expression(expression: &str, expected: &str) {
    assert_eq!(evaluate(expression), evaluate(expected), "{expression}");
}

#[test]
fn list_end_operations_and_splice_return_copies_and_removed_values() {
    for (expression, expected) in [
        ("List::push([], null)", "[null]"),
        ("List::push([1, 2], 3)", "[1, 2, 3]"),
        ("List::unshift([], 1)", "[1]"),
        ("List::unshift([2, 3], 1)", "[1, 2, 3]"),
        ("List::pop([1, null])", "['items' => [1], 'value' => null]"),
        (
            "List::shift([null, 2])",
            "['items' => [2], 'value' => null]",
        ),
        ("List::pop([1])", "['items' => [], 'value' => 1]"),
        ("List::shift([1])", "['items' => [], 'value' => 1]"),
        (
            "List::splice([1, 2, 3], 1, 1, [8, 9])",
            "['items' => [1, 8, 9, 3], 'removed' => [2]]",
        ),
        (
            "List::splice([1, 2, 3], 0, 3)",
            "['items' => [], 'removed' => [1, 2, 3]]",
        ),
        (
            "List::splice([1], 1, 0, [2])",
            "['items' => [1, 2], 'removed' => []]",
        ),
        ("List::splice([], 0, 0)", "['items' => [], 'removed' => []]"),
        (
            "List::splice([1, 2], 1, 0)",
            "['items' => [1, 2], 'removed' => []]",
        ),
        (
            "List::splice([1, 2], 0, 1)",
            "['items' => [2], 'removed' => [1]]",
        ),
    ] {
        assert_expression(expression, expected);
    }
}

#[test]
fn list_generation_search_and_uniqueness_are_strict_and_ordered() {
    for (expression, expected) in [
        ("List::reverse([1, 2, 3])", "[3, 2, 1]"),
        ("List::reverse([])", "[]"),
        ("List::chunk([1, 2, 3, 4, 5], 2)", "[[1, 2], [3, 4], [5]]"),
        ("List::chunk([1, 2], 5)", "[[1, 2]]"),
        ("List::chunk([], 2)", "[]"),
        ("List::chunk([1, 2], 1)", "[[1], [2]]"),
        ("List::fill(3, null)", "[null, null, null]"),
        ("List::fill(2, [1])", "[[1], [1]]"),
        ("List::fill(0, 1)", "[]"),
        ("List::search([null, 1, 1.0, '1', 1], 1)", "1"),
        ("List::search([null, 1], null)", "0"),
        ("List::search([1], 1.0)", "null"),
        ("List::search([], 1)", "null"),
        (
            "List::unique([1, 1.0, '1', true, null, 1, null])",
            "[1, 1.0, '1', true, null]",
        ),
        ("List::unique([[1], [1], [2]])", "[[1], [2]]"),
        (
            "List::unique([['a' => 1, 'b' => 2], ['b' => 2, 'a' => 1]])",
            "[['a' => 1, 'b' => 2]]",
        ),
        ("List::unique([])", "[]"),
        ("List::sum([])", "0"),
        ("List::product([])", "1"),
        ("List::sum([1, -2, 3])", "2"),
        ("List::product([2, -3, 4])", "-24"),
        ("List::sum([1.5, 2.5])", "4.0"),
        ("List::product([1.5, 2.0])", "3.0"),
        ("List::product([0, 9223372036854775807])", "0"),
    ] {
        assert_expression(expression, expected);
    }
}

#[test]
fn map_operations_preserve_typed_keys_and_duplicate_insertion_positions() {
    // Map equality ignores order, so compare keys and values explicitly.
    for (expression, keys, values) in [
        (
            "Map::merge([1 => 'a', '1' => 'b', 'z' => 0], ['1' => null, 'n' => 3, 1 => 'c'])",
            "[1, '1', 'z', 'n']",
            "['c', null, 0, 3]",
        ),
        (
            "Map::replace([1 => 'a', '1' => 'b'], ['1' => null, 'n' => 3])",
            "[1, '1']",
            "['a', null]",
        ),
        (
            "Map::combine([1, '1', 2, 1], ['a', 'b', 'c', 'd'])",
            "[1, '1', 2]",
            "['d', 'b', 'c']",
        ),
        (
            "Map::flip(['a' => 1, 'b' => '1', 'c' => 1])",
            "[1, '1']",
            "['c', 'b']",
        ),
        (
            "Map::reverse([1 => 'a', '1' => 'b', 5 => null])",
            "[5, '1', 1]",
            "[null, 'b', 'a']",
        ),
        ("Map::merge(Map::create(), ['a' => 1])", "['a']", "[1]"),
        ("Map::replace(Map::create(), ['a' => 1])", "[]", "[]"),
        ("Map::combine([], [])", "[]", "[]"),
        ("Map::flip(Map::create())", "[]", "[]"),
        ("Map::reverse(Map::create())", "[]", "[]"),
    ] {
        assert_expression(&format!("Map::keys({expression})"), keys);
        assert_expression(&format!("Map::values({expression})"), values);
    }
    for (expression, expected) in [
        ("Map::search([0 => null, 'zero' => null], null)", "0"),
        ("Map::search(['1' => 1, 1 => 1], 1)", "'1'"),
        ("Map::search([1 => 1], 1.0)", "null"),
        ("Map::search(Map::create(), null)", "null"),
    ] {
        assert_expression(expression, expected);
    }
}

#[test]
fn comparisons_preserve_left_order_duplicates_and_keys() {
    for (expression, expected) in [
        ("List::diff([3, 1, 3, 1.0, null], [1, null])", "[3, 3, 1.0]"),
        (
            "List::intersect([3, 1, 3, 1.0, null], [3, null])",
            "[3, 3, null]",
        ),
        ("List::diff([1], [])", "[1]"),
        ("List::intersect([1], [])", "[]"),
        ("List::diff([], [1])", "[]"),
        ("List::intersect([], [1])", "[]"),
        ("List::intersect([[1], [1], [2]], [[1]])", "[[1], [1]]"),
        (
            "Map::keys(Map::diff(['a' => 1, 2 => 1.0, 'b' => 1, 'c' => null], ['x' => 1]))",
            "[2, 'c']",
        ),
        (
            "Map::keys(Map::intersect(['a' => 1, 2 => 1.0, 'b' => 1, 'c' => null], ['x' => 1]))",
            "['a', 'b']",
        ),
        (
            "Map::keys(Map::diffKeys([1 => 'a', '1' => 'b', 'z' => 'c'], [1 => null]))",
            "['1', 'z']",
        ),
        (
            "Map::keys(Map::intersectKeys([1 => 'a', '1' => 'b', 'z' => 'c'], ['1' => null, 1 => 'x']))",
            "[1, '1']",
        ),
        (
            "Map::intersect(['a' => null], ['b' => null])",
            "['a' => null]",
        ),
        ("Map::diff(Map::create(), Map::create())", "Map::create()"),
        (
            "Map::intersectKeys(Map::create(), Map::create())",
            "Map::create()",
        ),
    ] {
        assert_expression(expression, expected);
    }
}

#[test]
fn columns_require_records_and_preserve_null_and_typed_duplicate_indexes() {
    let rows = "[['id' => 1, 'v' => 'first'], ['id' => '1', 'v' => null], ['id' => 2, 'v' => 'second'], ['id' => 1, 'v' => 'last']]";
    assert_expression(
        &format!("List::column({rows}, 'v')"),
        "['first', null, 'second', 'last']",
    );
    assert_expression(
        &format!("Map::keys(Map::column({rows}, 'v', 'id'))"),
        "[1, '1', 2]",
    );
    assert_expression(
        &format!("Map::values(Map::column({rows}, 'v', 'id'))"),
        "['last', null, 'second']",
    );
    assert_expression("List::column([[1 => null, '1' => 2]], 1)", "[null]");
    assert_expression("List::column([], 'v')", "[]");
    assert_expression("Map::column([], 'v', 'id')", "Map::create()");
}

#[test]
fn new_helpers_raise_catchable_errors_without_assigning_partial_results() {
    for (kind, expressions) in [
        (
            "RangeError",
            vec![
                "List::pop([])",
                "List::shift([])",
                "List::chunk([], 0)",
                "List::chunk([1], -1)",
                "List::fill(-1, null)",
                "List::splice([1], -1, 0)",
                "List::splice([1], 0, -1)",
                "List::splice([1], 2, 0)",
                "List::splice([1], 1, 1)",
                "List::splice([], 0, 9223372036854775807)",
                "Map::combine([1], [])",
                "List::column([Map::create()], 'v')",
                "Map::column([['v' => 1]], 'v', 'id')",
            ],
        ),
        (
            "TypeError",
            vec![
                "List::push([], 1, 2)",
                "List::unshift([])",
                "List::pop(Map::create())",
                "List::splice([], 0, 0, null)",
                "List::chunk([], 1.0)",
                "List::fill(1.0, 0)",
                "List::sum([1, 1.0])",
                "List::product([1.0, 1])",
                "List::sum([true])",
                "List::product([null])",
                "List::sum(['1'])",
                "List::sum([[]])",
                "Map::merge([], Map::create())",
                "Map::replace(Map::create(), [])",
                "Map::combine([true], [1])",
                "Map::combine([null], [1])",
                "Map::combine([1.0], [1])",
                "Map::flip(['x' => null])",
                "Map::flip(['x' => []])",
                "Map::flip(['x' => true])",
                "List::column([[]], 'v')",
                "List::column([1], 'v')",
                "List::column([], true)",
                "Map::column([['id' => null, 'v' => 1]], 'v', 'id')",
                "Map::column([], 'v', null)",
            ],
        ),
        (
            "ArithmeticError",
            vec![
                "List::sum([9223372036854775807, 1])",
                "List::sum([(-9223372036854775807 - 1), -1])",
                "List::product([9223372036854775807, 2])",
                "List::product([(-9223372036854775807 - 1), -1])",
                "List::sum([1.0e308, 1.0e308])",
                "List::product([1.0e308, 2.0])",
            ],
        ),
    ] {
        for expression in expressions {
            let mut runtime = runtime(None);
            execute(
                &mut runtime,
                &format!(
                    "$result = 'unchanged'; $caught = false; try {{ $result = {expression}; }} catch ({kind} $e) {{ $caught = true; }}"
                ),
            );
            assert_eq!(
                runtime.get_global("caught"),
                Some(&Value::Bool(true)),
                "{expression}"
            );
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into())),
                "{expression}"
            );
        }
    }
}

#[test]
fn generated_and_nested_results_do_not_alias_inputs_or_sibling_values() {
    let mut runtime = runtime(None);
    execute(
        &mut runtime,
        r#"
        $items = [[1], [2]];
        $pushed = List::push($items, [3]); $pushed[0][] = 9;
        $unshifted = List::unshift($items, [3]); $unshifted[1][] = 9;
        $popped = List::pop($items); $popped["items"][0][] = 9; $popped["value"][] = 9;
        $shifted = List::shift($items); $shifted["value"][] = 9;
        $spliced = List::splice($items, 0, 1, $items); $spliced["items"][0][] = 9; $spliced["removed"][0][] = 9;
        $chunked = List::chunk($items, 1); $chunked[0][0][] = 9;
        $filled = List::fill(2, $items); $filled[0][0][] = 9;
        $reversed = List::reverse($items); $reversed[0][] = 9;
        $unique = List::unique($items); $unique[0][] = 9;
        $diff = List::diff($items, []); $diff[0][] = 9;
        $intersect = List::intersect($items, $items); $intersect[0][] = 9;
        $map = ["a" => [1]];
        $merged = Map::merge($map, Map::create()); $merged["a"][] = 9;
        $replaced = Map::replace($map, Map::create()); $replaced["a"][] = 9;
        $combined = Map::combine(["a"], [$map]); $combined["a"]["a"][] = 9;
        $reversedMap = Map::reverse($map); $reversedMap["a"][] = 9;
        $rows = [["id" => "a", "v" => [1]]];
        $column = List::column($rows, "v"); $column[0][] = 9;
        $indexed = Map::column($rows, "v", "id"); $indexed["a"][] = 9;
    "#,
    );
    assert_eq!(runtime.get_global("items"), Some(&evaluate("[[1], [2]]")));
    assert_eq!(runtime.get_global("map"), Some(&evaluate("['a' => [1]]")));
    assert_eq!(
        runtime.get_global("rows"),
        Some(&evaluate("[['id' => 'a', 'v' => [1]]]"))
    );
    assert_eq!(
        runtime.get_global("filled"),
        Some(&evaluate("[[[1, 9], [2]], [[1], [2]]]"))
    );
}

#[test]
fn strict_helpers_keep_callable_and_object_identity() {
    let mut runtime = runtime(None);
    runtime
        .set_global("object", Value::NativeObject(Rc::new(Object)))
        .unwrap();
    runtime
        .set_global("other", Value::NativeObject(Rc::new(Object)))
        .unwrap();
    execute(
        &mut runtime,
        "function first() {} function second() {} $unique = List::unique([first, first, second, $object, $object, $other]); $index = List::search([second, first], first); $key = Map::search(['a' => $other, 'b' => $object], $object); $intersection = List::intersect([first, second, first], [first]);",
    );
    let Value::List(values) = runtime.get_global("unique").unwrap() else {
        panic!("list")
    };
    assert_eq!(values.len(), 4);
    assert_eq!(runtime.get_global("index"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("key"), Some(&Value::String("b".into())));
    let Value::List(values) = runtime.get_global("intersection").unwrap() else {
        panic!("list")
    };
    assert_eq!(values.len(), 2);
}
