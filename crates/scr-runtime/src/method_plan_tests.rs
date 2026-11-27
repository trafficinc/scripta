use super::{FIELDS, PARAMETERS, STATEMENTS};
use crate::*;
use std::cell::Cell;

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let (fast, mut reference) = crate::test_support::runtimes(options);
    reference.disable_method_plans = true;
    (fast, reference)
}

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let (mut fast, mut reference) = pair(options);
    let _ = crate::test_support::compare_with_recovery(
        &mut fast,
        &mut reference,
        source,
        "$recovered=7;",
    );
    fast
}

#[test]
fn scalar_methods_preserve_field_aliases_arguments_bound_receivers_and_return_checks() {
    let runtime = compare(
        "class C{$x:int=0;$y=1;\
         function next($delta:int):int{$this->x+=$delta;$this->y=$this->x*2;return $this->y+$delta;}\
         function valid($flag:bool):bool{return $flag && $this->x>0;}\
         function clear():void{$this->x=0;return;}}\
         $c=new C();$other=new C();$bound=$c->next;$a=$bound(3);$b=$other->next(1);\
         $d=$c->next(2);$flag=$c->valid(true);$seen=[$a,$b,$d,$c->x,$other->x,$flag];$c->clear();$cleared=$c->x;",
        RuntimeOptions::default(),
    );
    assert!(runtime.method_plans_executed >= 5);
    assert_eq!(
        runtime.get_global("seen"),
        Some(&Value::list(vec![
            Value::Int(9),
            Value::Int(3),
            Value::Int(12),
            Value::Int(5),
            Value::Int(1),
            Value::Bool(true)
        ]))
    );
    assert_eq!(runtime.get_global("cleared"), Some(&Value::Int(0)));
    for source in [
        "class C{$x=0;function f($x:int){$this->x=9;return $x;}}$c=new C();$c->f(true);",
        "class C{$x=0;function f():bool{$this->x=9;return $this->x;}}$c=new C();$c->f();",
        "class C{$x:int=0;function f(){$this->x=true;return $this->x;}}$c=new C();$c->f();",
        "class C{$x=0;function f(){$this->x=9;$this->x/=0;return $this->x;}}$c=new C();$seen='kept';try{$seen=$c->f();}catch(ArithmeticError $e){}$field=$c->x;",
        "class C{$x=9223372036854775807;function f(){$this->x+=1;return $this->x;}}$c=new C();$c->f();",
    ] {
        compare(source, RuntimeOptions::default());
    }
}

#[test]
fn literal_and_input_returns_preserve_types_field_updates_and_bound_receivers() {
    let source = "class C{$x:int=0;$flag:bool=false;\
        function store($n:int):int{$this->x=($n);return ($this->x);}\
        function setFlag($flag:bool):bool{$this->flag=$flag;return ($flag);}\
        function constant():int{return (42);}\
        function nothing(){return null;}}\
        $c=new C();$bound=$c->store;$a=$bound(7);$b=$c->setFlag(true);\
        $d=$c->constant();$e=$c->nothing();$f=$c->store(9);\
        $seen=[$a,$b,$d,$e,$f,$c->x,$c->flag];";
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    crate::test_support::compare_source(&mut fast, &mut reference, source).unwrap();
    assert_eq!(fast.method_plans_executed, 5);
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::list(vec![
            Value::Int(7),
            Value::Bool(true),
            Value::Int(42),
            Value::Null,
            Value::Int(9),
            Value::Int(9),
            Value::Bool(true),
        ]))
    );
    for expression in ["42", "true", "null", "$this->x"] {
        let source = format!(
            "class C{{$x=7;function invalid():string{{return {expression};}}}}\
             $c=new C();$seen='kept';$seen=$c->invalid();"
        );
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        assert!(
            crate::test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=1;",
            )
            .is_err()
        );
        assert_eq!(fast.get_global("seen"), Some(&Value::String("kept".into())));
        assert_eq!(fast.method_plans_executed, 1);
    }
}

#[test]
fn unsupported_method_states_fall_back_before_any_writes() {
    for source in [
        "class C{$x=0;$missing:int;function f(){$this->x=9;return $this->missing;}}$c=new C();try{$c->f();}catch(PropertyError $e){}$seen=$c->x;",
        "class C{$x=0;function f($x=2){$this->x=$x;return $this->x;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f(...$items){$this->x=$items[0];return $this->x;}}$c=new C();$seen=$c->f(3);",
        "class C{$x=0;function f(){return function(){return $this->x+=1;};}}$c=new C();$read=$c->f();$seen=$read();",
        "class C{$x=1.0;function f(){$this->x+=2.0;return $this->x;}}$c=new C();$seen=$c->f();",
        "class C extends Error{$x=0;function f(){$this->x+=1;return $this->x;}}$c=new C('message');$seen=$c->f();",
        "class C{$x=0;function f($callback){$this->x=9;return $callback();}}$c=new C();$seen=$c->f(function(){return 3;});",
    ] {
        let runtime = compare(source, RuntimeOptions::default());
        assert_eq!(runtime.method_plans_executed, 0, "{source}");
    }
    let runtime = compare(
        "class C{$x=0;function f(){$this->x+=1;return $this->x;}}\
         $old=new C();$others=[new C() for $i in [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17]];$seen=$old->f();",
        RuntimeOptions::default(),
    );
    assert_eq!(runtime.method_plans_executed, 0);
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(1)));
}

#[test]
fn method_plans_preserve_exact_limits_sources_partial_writes_and_recovery() {
    let source = "class C{$x:int=0;function next($delta:int):int{$this->x+=$delta;return $this->x;}}\
                $c=new C();$a=$c->next(1);$b=$c->next(2);$seen=$c->x;";
    for limit in 0..45 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 0..8 {
        compare(
            source,
            RuntimeOptions {
                max_scopes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 0..4 {
        compare(
            source,
            RuntimeOptions {
                recursion_limit: limit,
                ..RuntimeOptions::default()
            },
        );
    }
    let exact = size_of::<Value>() * 2 + 1;
    for bytes in [exact - 1, exact, exact + 1] {
        let runtime = compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(bytes),
                ..RuntimeOptions::default()
            },
        );
        assert_eq!(runtime.method_plans_executed, 0);
    }
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "class C{$x=0;function f(){$this->x/=0;return $this->x;}}$c=new C();",
    )
    .unwrap();
    assert!(crate::test_support::compare_source(&mut fast, &mut reference, "$c->f();").is_err());
}

#[derive(Debug)]
struct Proxy {
    object: Rc<dyn NativeObject>,
    casts: Rc<Cell<usize>>,
}
impl NativeObject for Proxy {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.casts.set(self.casts.get() + 1);
        self.object.as_any()
    }
    fn type_name(&self) -> &str {
        "Proxy"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        panic!("script downcast")
    }
}

#[test]
fn host_proxies_keep_observable_downcasts_and_cached_plans_retain_no_receivers() {
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    crate::test_support::compare_source(&mut fast,&mut reference,
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$warm=$c->next();").unwrap();
    let mut calls = Vec::new();
    let mut weak = Vec::new();
    for runtime in [&mut fast, &mut reference] {
        let Value::NativeObject(object) = runtime.get_global("c").unwrap().clone() else {
            panic!("object")
        };
        weak.push(Rc::downgrade(&object));
        let casts = Rc::new(Cell::new(0));
        runtime
            .set_global(
                "proxy",
                Value::NativeObject(Rc::new(Proxy {
                    object,
                    casts: casts.clone(),
                })),
            )
            .unwrap();
        calls.push(casts);
    }
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "$seen=$proxy->next();$seen=$proxy->next();",
    )
    .unwrap();
    assert_eq!(calls[0].get(), calls[1].get());
    assert!(calls[0].get() > 0);
    crate::test_support::compare_source(&mut fast, &mut reference, "$c=null;$proxy=null;").unwrap();
    for runtime in [&mut fast, &mut reference] {
        runtime.collect_garbage();
    }
    assert!(weak.iter().all(|object| object.upgrade().is_none()));
}

#[test]
fn method_compilation_bounds_fall_back_without_partial_execution() {
    for parameters in [PARAMETERS, PARAMETERS + 1] {
        let source = format!(
            "class C{{{}function f({}){{{}return $this->f7;}}}}$c=new C();$seen=$c->f({});",
            (0..FIELDS)
                .map(|i| format!("$f{i}={i};"))
                .collect::<String>(),
            (0..parameters)
                .map(|i| format!("$p{i}"))
                .collect::<Vec<_>>()
                .join(","),
            (0..FIELDS)
                .map(|i| format!("$this->f{i}+=$p{i};"))
                .collect::<String>(),
            (1..=parameters)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let runtime = compare(&source, RuntimeOptions::default());
        assert_eq!(
            runtime.method_plans_executed,
            usize::from(parameters == PARAMETERS)
        );
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(15)));
    }
    for fields in [FIELDS, FIELDS + 1] {
        let source = format!(
            "class C{{{}function f(){{{}return $this->f0;}}}}$c=new C();$seen=$c->f();",
            (0..fields).map(|i| format!("$f{i}=0;")).collect::<String>(),
            (0..fields)
                .map(|i| format!("$this->f{i}+=1;"))
                .collect::<String>()
        );
        let runtime = compare(&source, RuntimeOptions::default());
        assert_eq!(runtime.method_plans_executed, usize::from(fields == FIELDS));
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(1)));
    }
    for statements in [STATEMENTS, STATEMENTS + 1] {
        let source = format!(
            "class C{{$x=0;function f(){{{}return $this->x;}}}}$c=new C();$seen=$c->f();",
            "$this->x+=1;".repeat(statements - 1)
        );
        let runtime = compare(&source, RuntimeOptions::default());
        assert_eq!(
            runtime.method_plans_executed,
            usize::from(statements == STATEMENTS)
        );
        assert_eq!(
            runtime.get_global("seen"),
            Some(&Value::Int((statements - 1) as i64))
        );
    }
}

#[test]
fn cancellation_after_method_entry_unwinds_the_planned_body() {
    let source = "$seen=$c->next();";
    let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let StatementKind::Expression(Expr {
        kind: ExprKind::Assignment(_, _, expression),
        ..
    }) = &program.statements[0].kind
    else {
        panic!("assignment")
    };
    let flag = Arc::new(AtomicBool::new(false));
    let (mut fast, mut reference) = pair(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..RuntimeOptions::default()
    });
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();",
    )
    .unwrap();
    flag.store(true, Ordering::Relaxed);
    let result = crate::test_support::compare_execution(
        &mut fast,
        &mut reference,
        source,
        |runtime, output| match runtime.evaluate(expression, output) {
            Err(Signal::Interrupted(error)) => Err(*error),
            _ => panic!("cancellation must interrupt the method body"),
        },
    );
    assert!(result.unwrap_err().message.contains("interrupted"));
    assert_eq!(fast.method_plans_executed, 1);
    flag.store(false, Ordering::Relaxed);
    crate::test_support::compare_source(&mut fast, &mut reference, "$seen=$c->x;").unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(0)));
}
