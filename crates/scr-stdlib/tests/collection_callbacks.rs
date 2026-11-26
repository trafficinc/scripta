use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeSignature, NativeType, Runtime,
    RuntimeOptions, Value,
};
use std::{
    cell::Cell,
    mem::size_of,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
fn runtime(options: RuntimeOptions) -> Runtime {
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register(&mut rt).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) {
    rt.execute_source(source, &mut Vec::new()).unwrap();
}
fn evaluate(rt: &mut Runtime, expression: &str) -> Value {
    run(rt, &format!("$result={expression};"));
    rt.get_global("result").unwrap().clone()
}
fn failure(rt: &mut Runtime, expression: &str, kind: &str) {
    rt.set_global("result", Value::Int(42)).unwrap();
    let error = rt
        .execute_source(&format!("$result={expression};"), &mut Vec::new())
        .unwrap_err();
    assert!(
        error.message.starts_with(&format!("{kind}:")),
        "{expression}: {error}"
    );
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
}
fn ints(items: &[i64]) -> Value {
    Value::List(items.iter().copied().map(Value::Int).collect())
}

#[test]
fn eager_list_helpers_reuse_named_native_and_captured_functions() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        function square($n:int):int { return $n*$n; }
        function makeOffset($amount) { return function($value) { return $value+$amount; }; }
        $items=[-2,0,3]; $squared=List::map($items,square);
        $offset=List::map($items,makeOffset(4));
        $trimmed=List::map([" a ","b\n"],function($v){return String::trim($v);});
        $positive=List::filter($items,function($x):bool { return $x>0; });
        $total=List::reduce($items,function($acc,$value){return $acc+$value;},10);
        $nested=List::map([[1],[2]],function($row){ $row[]=9; return $row; });
        $nativeStrings=List::map([1,2],str);
        $closures=List::map([1,2],function($value){return function(){return $value;};});
        $retained=[$closures[0](),$closures[1]()];
        $empty=List::reduce([],function(){throw 'not called';},["seed"=>null]);
    "#,
    );
    for (name, value) in [
        ("items", ints(&[-2, 0, 3])),
        ("squared", ints(&[4, 0, 9])),
        ("offset", ints(&[2, 4, 7])),
        ("positive", ints(&[3])),
        ("total", Value::Int(11)),
        (
            "trimmed",
            Value::List(vec![Value::String("a".into()), Value::String("b".into())]),
        ),
    ] {
        assert_eq!(rt.get_global(name), Some(&value), "{name}");
    }
    assert_eq!(
        rt.get_global("nativeStrings"),
        Some(&Value::List(vec![
            Value::String("1".into()),
            Value::String("2".into())
        ]))
    );
    assert_eq!(rt.get_global("retained"), Some(&ints(&[1, 2])));
    rt.set_global("many", ints(&(0..512).collect::<Vec<_>>()))
        .unwrap();
    run(
        &mut rt,
        "$survivors=List::map($many,function($value){return function(){return $value;};});",
    );
    rt.collect_garbage();
    run(
        &mut rt,
        "$afterCollection=[$survivors[0](),$survivors[511]()];",
    );
    assert_eq!(rt.get_global("afterCollection"), Some(&ints(&[0, 511])));
    assert_eq!(
        rt.get_global("nested"),
        Some(&Value::List(vec![ints(&[1, 9]), ints(&[2, 9])]))
    );
    assert_eq!(
        rt.get_global("empty"),
        Some(&Value::Map(vec![(
            MapKey::String("seed".into()),
            Value::Null
        )]))
    );
}

#[test]
fn map_callbacks_keep_typed_keys_order_nulls_and_empty_map_identity() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        $input=[1=>null,"1"=>2,"last"=>3];
        $mapped=Map::map($input,function($value,$key){return type($key);});
        $filtered=Map::filter($input,function($value,$key):bool {return $key===1 || $key==="last";});
        $keys=Map::reduce($input,function($acc,$value,$key){$acc[]=$key;return $acc;},[]);
        $none=Map::filter($input,function($value,$key){return false;});
        $empty=Map::map(Map::create(),function(){throw 'not called';});
    "#,
    );
    assert_eq!(
        rt.get_global("mapped"),
        Some(&Value::Map(vec![
            (MapKey::Int(1), Value::String("int".into())),
            (MapKey::String("1".into()), Value::String("string".into())),
            (
                MapKey::String("last".into()),
                Value::String("string".into())
            )
        ]))
    );
    assert_eq!(
        rt.get_global("filtered"),
        Some(&Value::Map(vec![
            (MapKey::Int(1), Value::Null),
            (MapKey::String("last".into()), Value::Int(3))
        ]))
    );
    let Some(Value::Map(mapped)) = rt.get_global("mapped") else {
        panic!()
    };
    assert_eq!(
        mapped
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        vec![
            MapKey::Int(1),
            MapKey::String("1".into()),
            MapKey::String("last".into())
        ]
    );
    let Some(Value::Map(filtered)) = rt.get_global("filtered") else {
        panic!()
    };
    assert_eq!(
        filtered
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        vec![MapKey::Int(1), MapKey::String("last".into())]
    );
    assert_eq!(
        rt.get_global("keys"),
        Some(&Value::List(vec![
            Value::Int(1),
            Value::String("1".into()),
            Value::String("last".into())
        ]))
    );
    assert_eq!(rt.get_global("none"), Some(&Value::Map(vec![])));
    assert_eq!(rt.get_global("empty"), Some(&Value::Map(vec![])));
}

#[test]
fn sort_by_computes_keys_once_in_input_order_and_preserves_ties_both_directions() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        function keyTracker() {
            $seen=[];
            return [function($row){$seen[]=$row["id"];return $row["rank"];},function(){return $seen;}];
        }
        $rows=[["id"=>"a","rank"=>2],["id"=>"b","rank"=>1],["id"=>"c","rank"=>2]];
        $tracker=keyTracker();$sorted=List::sortBy($rows,$tracker[0]);$seen=$tracker[1]();
        $ascending=List::column($sorted,"id");
        $descending=List::column(List::sortBy($rows,function($row){return $row["rank"];},true),"id");
        $words=List::sortBy(["b","A","a"],function($v){return String::lower($v);});
    "#,
    );
    let strings =
        |items: &[&str]| Value::List(items.iter().map(|s| Value::String((*s).into())).collect());
    assert_eq!(rt.get_global("seen"), Some(&strings(&["a", "b", "c"])));
    assert_eq!(rt.get_global("ascending"), Some(&strings(&["b", "a", "c"])));
    assert_eq!(
        rt.get_global("descending"),
        Some(&strings(&["a", "c", "b"]))
    );
    assert_eq!(rt.get_global("words"), Some(&strings(&["A", "a", "b"])));
    rt.set_global(
        "zeros",
        Value::List(vec![Value::Float(-0.0), Value::Float(0.0)]),
    )
    .unwrap();
    let Value::List(zeros) = evaluate(
        &mut rt,
        "List::sortBy($zeros,function($v){return $v;},true)",
    ) else {
        panic!()
    };
    assert!(matches!(zeros[0],Value::Float(value) if value.is_sign_negative()));
}

#[test]
fn comparator_sort_is_stable_uses_integer_sign_and_handles_inconsistent_order_without_panicking() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        function compare($a,$b) { if($a["k"]<$b["k"]) return -9223372036854775807-1; if($a["k"]>$b["k"]) return 9223372036854775807; return 0; }
        $rows=[["k"=>2,"id"=>"a"],["k"=>1,"id"=>"b"],["k"=>2,"id"=>"c"]];
        $sorted=List::column(List::sortWith($rows,compare),"id");
        $descending=List::column(List::sortWith($rows,compare,true),"id");
        $numbers=[1,2,3,4,5,6,7,8,9];
        $inconsistent=List::sortWith($numbers,function($a,$b){return 1;});
        $permutation=List::sort($inconsistent);
    "#,
    );
    assert_eq!(rt.get_global("permutation"), rt.get_global("numbers"));
    assert_eq!(
        rt.get_global("sorted"),
        Some(&Value::List(
            ["b", "a", "c"].map(|s| Value::String(s.into())).to_vec()
        ))
    );
    assert_eq!(
        rt.get_global("descending"),
        Some(&Value::List(
            ["a", "c", "b"].map(|s| Value::String(s.into())).to_vec()
        ))
    );
}

#[test]
fn empty_singleton_and_callback_type_contracts_are_explicit() {
    let mut rt = runtime(Default::default());
    for expression in [
        "List::map([],function(){throw 1;})",
        "List::filter([],function(){throw 1;})",
        "List::sortBy([],function(){throw 1;})",
        "List::sortWith([],function(){throw 1;})",
    ] {
        assert_eq!(evaluate(&mut rt, expression), ints(&[]));
    }
    assert_eq!(
        evaluate(&mut rt, "List::sortWith([null],function(){throw 1;})"),
        Value::List(vec![Value::Null])
    );
    for expression in [
        "List::map([],1)",
        "List::filter([1],function($x){return 1;})",
        "Map::filter(['a'=>1],function($v,$k){return null;})",
        "List::reduce([],function($a,$b){return $a;})",
        "List::map([1],function(){return 1;})",
        "List::sortBy([1],function($v){return true;})",
        "List::sortBy([1,2],function($v){if($v===1)return 1;return '2';})",
        "List::sortWith([1,2],function($a,$b){return 1.0;})",
        "List::sortWith([1,2],function($a,$b){return false;})",
        "List::sortBy([1],function($x){return $x;},null)",
    ] {
        failure(&mut rt, expression, "TypeError");
    }
}

#[test]
fn snapshots_reentrancy_and_shared_object_side_effects_follow_existing_value_semantics() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        function changedInput() {
            $items=[1,2,3];
            $mapped=List::map($items,function($v){$items=[99];return $v*2;});
            return [$items,$mapped];
        }
        $snapshot=changedInput();
        $nested=List::map([[1,2],[3]],function($row){return List::reduce(List::map($row,function($v){return $v*2;}),function($a,$b){return $a+$b;},0);});
        class Box { $value=0; }
        $box=new Box(); $objects=[$box];
        $mapped=List::map($objects,function($item){$item->value=7;return $item;});
        $shared=$box->value;
        function sideEffects() {
            $count=0; $result=42;
            try {$result=List::map([1,2,3],function($x){$count+=1;if($x===2)throw 9;return $x;});}catch(int $e){}
            return [$count,$result];
        }
        $effects=sideEffects();
    "#,
    );
    assert_eq!(
        rt.get_global("snapshot"),
        Some(&Value::List(vec![ints(&[99]), ints(&[2, 4, 6])]))
    );
    assert_eq!(rt.get_global("nested"), Some(&ints(&[6, 6])));
    assert_eq!(rt.get_global("shared"), Some(&Value::Int(7)));
    assert_eq!(rt.get_global("effects"), Some(&ints(&[2, 42])));
}

#[test]
fn callback_failures_keep_original_typed_throws_sources_and_stack_frames() {
    let mut rt = runtime(Default::default());
    let source = "function broken($value) { return 1 / 0; }\n$result=List::map([1],broken);";
    rt.set_global("result", Value::Int(42)).unwrap();
    let error = rt.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with("ArithmeticError:"));
    assert_eq!(error.span.start, source.find("1 / 0").unwrap());
    for function in ["broken", "List::map"] {
        assert!(
            error.trace.iter().any(|frame| frame.function == function),
            "{error:?}"
        );
    }
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
    run(
        &mut rt,
        "$caught=0;try {List::sortWith([1,2],function($a,$b){throw 17;});}catch(int $e){$caught=$e;}",
    );
    assert_eq!(rt.get_global("caught"), Some(&Value::Int(17)));
    rt.execute_source(
        "$result=List::map([1],function($x){exit 7;});",
        &mut Vec::new(),
    )
    .map(|code| assert_eq!(code, 7))
    .unwrap();
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
}

#[test]
fn exact_output_key_cache_and_depth_limits_fail_without_partial_assignments() {
    let slot = size_of::<Value>();
    let text = "x".repeat(60);
    let exact = 3 * slot + 2 * text.len();
    for maximum in [exact - 1, exact, exact + 1] {
        let mut rt = runtime(RuntimeOptions {
            max_value_bytes: Some(maximum),
            ..Default::default()
        });
        let expression = format!("List::map([1,2],function($x){{return '{text}';}})");
        if maximum < exact {
            failure(&mut rt, &expression, "ResourceLimitError");
        } else {
            assert_eq!(
                evaluate(&mut rt, &expression),
                Value::List(vec![
                    Value::String(text.clone()),
                    Value::String(text.clone())
                ])
            );
        }
        let expression = format!("List::sortBy([2,1],function($x){{return '{text}';}})");
        if maximum < exact {
            failure(&mut rt, &expression, "ResourceLimitError");
        } else {
            assert_eq!(evaluate(&mut rt, &expression), ints(&[2, 1]));
        }
    }
    let mut rt = runtime(Default::default());
    let mut deep = Value::Int(1);
    for _ in 0..128 {
        deep = Value::List(vec![deep]);
    }
    rt.register(
        "Test::deep",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Any),
            move |_, _| Ok(deep.clone()),
        ),
    )
    .unwrap();
    let callback = rt.get_callable("Test::deep").unwrap();
    rt.set_global("deep", callback).unwrap();
    failure(&mut rt, "List::map([1],$deep)", "ResourceLimitError");
}

#[test]
fn instruction_scope_recursion_cancellation_and_foreign_owner_failures_are_not_swallowed() {
    let mut rt = runtime(RuntimeOptions {
        instruction_limit: Some(60),
        ..Default::default()
    });
    rt.set_global("items", ints(&(0..100).collect::<Vec<_>>()))
        .unwrap();
    rt.set_global("result", Value::Int(42)).unwrap();
    let error = rt
        .execute_source(
            "try {$result=List::map($items,function($x){return $x;});}catch(Error $e){}",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.contains("instruction"));
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
    let mut scoped = runtime(RuntimeOptions {
        max_scopes: Some(1),
        ..Default::default()
    });
    scoped.set_global("result", Value::Int(42)).unwrap();
    let error = scoped
        .execute_source(
            "$result=List::map([1],function($x){return $x;});",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.contains("scope"));
    assert_eq!(scoped.get_global("result"), Some(&Value::Int(42)));
    let mut rt = runtime(RuntimeOptions {
        recursion_limit: 4,
        ..Default::default()
    });
    run(&mut rt, "function again($x){return List::map([$x],again);}");
    assert!(
        rt.execute_source(
            "try {List::map([1],again);}catch(Error $e){}",
            &mut Vec::new()
        )
        .is_err()
    );
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = runtime(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    rt.register(
        "Test::cancel",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Int),
            move |_, args| {
                flag.store(true, Ordering::Relaxed);
                Ok(args[0].clone())
            },
        ),
    )
    .unwrap();
    let callback = rt.get_callable("Test::cancel").unwrap();
    rt.set_global("cancel", callback).unwrap();
    assert!(
        rt.execute_source(
            "try {List::map([1,2],$cancel);}catch(Error $e){}",
            &mut Vec::new()
        )
        .is_err()
    );
    let mut foreign = runtime(Default::default());
    run(&mut foreign, "function foreign($v){return $v;}");
    let callable = foreign.get_callable("foreign").unwrap();
    let mut rt = runtime(Default::default());
    rt.set_global("callback", callable).unwrap();
    assert!(
        rt.execute_source(
            "try {List::map([1],$callback);}catch(Error $e){}",
            &mut Vec::new()
        )
        .unwrap_err()
        .message
        .contains("different runtime")
    );
}

#[derive(Debug)]
struct MutablePayload(Rc<Cell<usize>>);
impl NativeObject for MutablePayload {
    fn type_name(&self) -> &str {
        "MutablePayload"
    }
    fn payload_bytes(&self) -> usize {
        self.0.get()
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
}
#[test]
fn retained_outputs_are_rechecked_after_callback_mutates_native_payload() {
    let size = Rc::new(Cell::new(0));
    let object = Rc::new(MutablePayload(size.clone()));
    let mut rt = runtime(RuntimeOptions {
        max_value_bytes: Some(3 * size_of::<Value>()),
        ..Default::default()
    });
    rt.register(
        "Test::payload",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Any),
            move |_, args| {
                if args[0] == Value::Int(1) {
                    Ok(Value::NativeObject(object.clone()))
                } else {
                    object.0.set(1000);
                    Ok(Value::Null)
                }
            },
        ),
    )
    .unwrap();
    let callback = rt.get_callable("Test::payload").unwrap();
    rt.set_global("payload", callback).unwrap();
    failure(&mut rt, "List::map([1,2],$payload)", "ResourceLimitError");
    assert_eq!(size.get(), 1000);
}
#[derive(Debug)]
struct Counted(Arc<AtomicUsize>);
impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
impl NativeObject for Counted {
    fn type_name(&self) -> &str {
        "Counted"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
}
#[test]
fn partial_outputs_release_owned_native_values_after_callback_failure() {
    let live = Arc::new(AtomicUsize::new(0));
    let count = live.clone();
    let mut rt = runtime(Default::default());
    rt.register(
        "Test::resource",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Any),
            move |_, args| {
                if args[0] == Value::Int(2) {
                    return Err(NativeError::new("TestError", "intentional failure"));
                }
                count.fetch_add(1, Ordering::Relaxed);
                Ok(Value::NativeObject(Rc::new(Counted(count.clone()))))
            },
        ),
    )
    .unwrap();
    let callback = rt.get_callable("Test::resource").unwrap();
    rt.set_global("resource", callback).unwrap();
    failure(&mut rt, "List::map([1,2],$resource)", "TestError");
    assert_eq!(live.load(Ordering::Relaxed), 0);
    for expression in [
        "Map::map(['a'=>1,'b'=>2],function($v,$key){return Test::resource($v);})",
        "List::reduce([1,2],function($acc,$v){return Test::resource($v);},null)",
        "Map::reduce(['a'=>1,'b'=>2],function($acc,$v,$key){return Test::resource($v);},null)",
    ] {
        failure(&mut rt, expression, "TestError");
        assert_eq!(live.load(Ordering::Relaxed), 0, "{expression}");
    }
}

#[test]
fn owned_callback_arguments_keep_nested_snapshots_and_seed_values() {
    let mut rt = runtime(Default::default());
    run(
        &mut rt,
        r#"
        $input=["a"=>["rank"=>2,"nested"=>[1]],7=>["rank"=>1,"nested"=>[2]]];
        $mapped=Map::map($input,function($row,$key){$row["nested"][]=9;return $row;});
        $filtered=Map::filter($input,function($row,$key){$row["nested"][]=8;return true;});
        $sorted=List::sortWith([$input["a"],$input[7]],function($a,$b){
            $a["nested"][]=8;$b["nested"][]=9;return $a["rank"]-$b["rank"];
        });
        $seed=["values"=>[]];
        $reduced=List::reduce([1,2],function($acc,$v){$acc["values"][]=$v;return $acc;},$seed);
        $mapReduced=Map::reduce(["a"=>1,7=>2],function($acc,$v,$key){$acc["values"][]=$v;return $acc;},$seed);
        $recorded=[$input["a"]["nested"],$input[7]["nested"],
            $mapped["a"]["nested"],$mapped[7]["nested"],
            $filtered["a"]["nested"],$filtered[7]["nested"],
            $sorted[0]["nested"],$sorted[1]["nested"],
            $seed["values"],$reduced["values"],$mapReduced["values"]];
    "#,
    );
    assert_eq!(
        rt.get_global("recorded"),
        Some(&Value::List(vec![
            ints(&[1]),
            ints(&[2]),
            ints(&[1, 9]),
            ints(&[2, 9]),
            ints(&[1]),
            ints(&[2]),
            ints(&[2]),
            ints(&[1]),
            ints(&[]),
            ints(&[1, 2]),
            ints(&[1, 2]),
        ]))
    );
    rt.set_global("many", ints(&(0..64).collect::<Vec<_>>()))
        .unwrap();
    run(
        &mut rt,
        r#"
        $closureAccumulator=List::reduce($many,function($acc,$value){
            $acc[]=function(){return $value;};return $acc;
        },[]);
        $mappedClosures=Map::map(["a"=>1,7=>2],function($value,$key){
            return function(){return $value;};
        });
    "#,
    );
    rt.collect_garbage();
    run(
        &mut rt,
        "$closureResults=[$closureAccumulator[0](),$closureAccumulator[63](),$mappedClosures['a'](),$mappedClosures[7]()];",
    );
    assert_eq!(rt.get_global("closureResults"), Some(&ints(&[0, 63, 1, 2])));
}
