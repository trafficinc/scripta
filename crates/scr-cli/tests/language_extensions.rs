use scr_runtime::{ContextOptions, NativeObject, Runtime, RuntimeOptions, Value};
use std::{
    fs,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn runtime() -> Runtime {
    let mut rt = Runtime::new();
    scr_stdlib::register(&mut rt).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) -> Result<(), scr_runtime::RuntimeError> {
    rt.execute_source(source, &mut Vec::new()).map(|_| ())
}

#[test]
fn arrays_support_negative_reads_writes_slices_membership_and_unpacked_calls() {
    let mut rt = runtime();
    run(&mut rt, "$a=[0,1,2,3,4]; $last=$a[-1]; $a[-2]=30; $slice=$a[1:4]; $reverse=$a[::-1]; $step=$a[::2]; $tail=$a[-2:]; $empty=$a[4:1]; $explicit=$a[:-1:-1]; $map=[1=>'integer', '1'=>'string']; $yes=1 in $a; $key='1' in $map; $text='λ' in 'aλb'; function add($x,$y) {return $x+$y;} $sum=add(...[2,3]); $combined=[-1,...$a,5];").unwrap();
    assert_eq!(rt.get_global("last"), Some(&Value::Int(4)));
    assert_eq!(rt.get_global("sum"), Some(&Value::Int(5)));
    assert_eq!(
        rt.get_global("slice"),
        Some(&Value::list(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(30)
        ]))
    );
    assert_eq!(
        rt.get_global("reverse"),
        Some(&Value::list(vec![
            Value::Int(4),
            Value::Int(30),
            Value::Int(2),
            Value::Int(1),
            Value::Int(0)
        ]))
    );
    assert_eq!(
        rt.get_global("step"),
        Some(&Value::list(vec![
            Value::Int(0),
            Value::Int(2),
            Value::Int(4)
        ]))
    );
    assert_eq!(rt.get_global("explicit"), Some(&Value::list(vec![])));
    assert_eq!(rt.get_global("key"), Some(&Value::Bool(true)));
    assert_eq!(rt.get_global("text"), Some(&Value::Bool(true)));
    for source in [
        "$a[-6];",
        "$a[-6]=1;",
        "$a[::0];",
        "$a[1.0:];",
        "$a=[...1];",
    ] {
        assert!(run(&mut rt, source).is_err(), "{source}");
    }
}

#[test]
fn comprehension_bindings_do_not_leak_and_destructuring_failure_is_atomic() {
    let mut rt = runtime();
    run(&mut rt, "$x='outer'; $a=[$x*$x for $x in [1,2,3] if $x>1]; [$first,...$rest]=$a; $fns=[function () {return $x;} for $x in [1,2]]; $captured=[$fns[0](),$fns[1]()]; $before='kept'; try {[$before,$other]=[1];} catch (RangeError $e) {} ").unwrap();
    assert_eq!(rt.get_global("x"), Some(&Value::String("outer".into())));
    assert_eq!(rt.get_global("first"), Some(&Value::Int(4)));
    assert_eq!(
        rt.get_global("rest"),
        Some(&Value::list(vec![Value::Int(9)]))
    );
    assert_eq!(
        rt.get_global("captured"),
        Some(&Value::list(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(rt.get_global("before"), Some(&Value::String("kept".into())));
    assert!(rt.get_global("other").is_none());
    assert!(
        run(&mut rt, "$bad=[$v for $v in [1] if 1];")
            .unwrap_err()
            .message
            .contains("bool")
    );
}

#[test]
fn classes_have_typed_fields_reference_identity_methods_and_private_per_instance_defaults() {
    let mut rt = runtime();
    run(&mut rt, "class User { $name:string; $items=[]; function __construct($name:string) {$this->name=$name;} function greet():string {return 'Hello, '.$this->name;} function callback() {return function () {return $this->name;};} } $a=new User('Ada'); $b=$a; $b->name='Grace'; $c=new User('Other'); $a->items[]=1; $message=$a->greet(); $same=$a===$b; $different=$a!==$c; $cb=$a->callback(); $method=$a->greet; $values=[$u->name for $u in [$a,$c]]; try {$a->name=123;} catch (TypeError $e) {} $name=$a->name; $independent=$c->items;").unwrap();
    assert_eq!(
        rt.get_global("message"),
        Some(&Value::String("Hello, Grace".into()))
    );
    assert_eq!(rt.get_global("same"), Some(&Value::Bool(true)));
    assert_eq!(rt.get_global("different"), Some(&Value::Bool(true)));
    assert_eq!(rt.get_global("name"), Some(&Value::String("Grace".into())));
    assert_eq!(rt.get_global("independent"), Some(&Value::list(vec![])));
    rt.collect_garbage();
    run(&mut rt, "$from_callback=$cb(); $bound=$method();").unwrap();
    assert_eq!(
        rt.get_global("from_callback"),
        Some(&Value::String("Grace".into()))
    );
    assert_eq!(
        rt.get_global("bound"),
        Some(&Value::String("Hello, Grace".into()))
    );
    assert!(
        run(&mut rt, "$a->missing=1;")
            .unwrap_err()
            .message
            .contains("unknown field")
    );
    assert!(
        run(
            &mut rt,
            "class Empty {$value:int;} $e=new Empty(); $e->value;"
        )
        .unwrap_err()
        .message
        .contains("uninitialized")
    );
    run(
        &mut rt,
        "class Keywords {$class='safe'; $in=1;} $k=new Keywords(); $keyword=$k->class;",
    )
    .unwrap();
    assert_eq!(
        rt.get_global("keyword"),
        Some(&Value::String("safe".into()))
    );
}

#[test]
fn module_classes_keep_source_identity_and_same_named_types_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        fs::write(dir.path().join(format!("{name}.scripta")), "class User {$name='module'; function value():User {return $this;} } function accept($user:User) {return $user->name;}").unwrap();
    }
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            script_path: Some(dir.path().join("main.scripta")),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut rt).unwrap();
    run(&mut rt,"require 'a.scripta' as A; require 'b.scripta' as B; $a=new A::User(); $b=new B::User(); $result=A::accept($a); $copy=$a->value(); try { A::accept($b); } catch (TypeError $e) {$rejected=true;} function takes($user:A::User) {return $user->name;} $qualified=takes($a);").unwrap();
    assert_eq!(
        rt.get_global("result"),
        Some(&Value::String("module".into()))
    );
    // Catch bindings have lexical scope, so verify the mismatch directly too.
    assert!(
        run(&mut rt, "A::accept($b);")
            .unwrap_err()
            .message
            .contains("TypeError")
    );
    assert_eq!(
        rt.get_global("qualified"),
        Some(&Value::String("module".into()))
    );
}

#[derive(Debug)]
struct Probe(Arc<AtomicUsize>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Probe"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("unknown".into())
    }
}

#[test]
fn object_cycles_and_bound_method_cycles_are_collected_under_scope_pressure() {
    let dropped = Arc::new(AtomicUsize::new(0));
    let counter = dropped.clone();
    let mut rt = Runtime::with_options(RuntimeOptions {
        max_scopes: Some(12),
        ..Default::default()
    });
    rt.register(
        "probe",
        scr_runtime::NativeFunction::new(
            scr_runtime::NativeSignature::new(vec![], scr_runtime::NativeType::Any),
            move |_, _| Ok(Value::NativeObject(Rc::new(Probe(counter.clone())))),
        ),
    )
    .unwrap();
    run(&mut rt,"class Node {$next; $callback; $payload; function __construct() {$this->payload=probe();} function get() {return $this;} function capture() {return function () {return $this;};} } for ($i=0;$i<100;$i+=1) {$node=new Node(); $node->next=$node; if ($i%2===0) {$node->callback=$node->get;} else {$node->callback=$node->capture();} $node=null;}").unwrap();
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 100);
    run(&mut rt, "$node=new Node(); $method=$node->get; $node=null;").unwrap();
    let method = rt.get_global("method").unwrap().clone();
    run(&mut rt, "$method=null;").unwrap();
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 100);
    rt.invoke(method.clone(), &[], &mut Vec::new()).unwrap();
    drop(method);
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 101);
}

#[test]
fn new_syntax_formats_idempotently_and_rejects_invalid_forms() {
    let source = "class User{$name:string;function __construct($name:string){$this->name=$name;}function get(){return $this->name;}}$users=[new User('Ada')];$names=[$u->get() for $u in $users if $u->name in ['Ada']];[$first,...$rest]=[...$names,'last'];$slice=$users[::-1];";
    let formatted = scr_cli::formatter::format(source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    for source in [source, &formatted] {
        run(&mut runtime(), source).unwrap();
    }
    for source in [
        "class A {$x; $x;}",
        "class A {function x(){} function x(){}}",
        "function f() {class A {}}",
        "[$x,...$rest,$last]=[1,2];",
        "$a[::0]=[];",
    ] {
        assert!(scr_cli::pipeline::check(source).is_err(), "{source}");
    }
}

#[test]
fn object_value_limits_are_exact_and_failed_updates_and_constructors_preserve_bindings() {
    let exact = 2 * std::mem::size_of::<Value>() + 1 + 3;
    let mut rt = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(exact),
        ..Default::default()
    });
    run(&mut rt, "class Box {$x='abc';} $box=new Box();").unwrap();
    let value = rt.get_global("box").unwrap().clone();
    assert!(
        run(&mut rt, "$box->x='abcd';")
            .unwrap_err()
            .message
            .contains("value byte limit")
    );
    run(&mut rt, "$previous=$box->x;").unwrap();
    assert_eq!(
        rt.get_global("previous"),
        Some(&Value::String("abc".into()))
    );
    assert_eq!(rt.get_global("box"), Some(&value));
    let mut over = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(exact - 1),
        ..Default::default()
    });
    assert!(
        run(
            &mut over,
            "class Box {$x='abc';} $box='kept'; $box=new Box();"
        )
        .is_err()
    );
    assert_eq!(over.get_global("box"), Some(&Value::String("kept".into())));
    let mut rt = runtime();
    run(&mut rt,"class Fails {$x; function __construct() {throw 'failed';}} $saved='kept'; try {$saved=new Fails();} catch ($e) {} $after=1;").unwrap();
    assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
    assert_eq!(rt.get_global("after"), Some(&Value::Int(1)));
}

#[test]
fn array_and_construction_limits_cancel_and_foreign_objects_are_rejected() {
    let mut limited = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(20),
        ..Default::default()
    });
    assert!(
        run(
            &mut limited,
            "$result=[$x for $x in [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20]];"
        )
        .unwrap_err()
        .message
        .contains("instruction limit")
    );
    assert!(limited.get_global("result").is_none());
    let mut recursive = runtime();
    assert!(
        run(
            &mut recursive,
            "class Loop {$value=new Loop();} $value=new Loop();"
        )
        .unwrap_err()
        .message
        .contains("depth")
    );
    let mut source = runtime();
    run(&mut source, "class User {$name='Ada';} $user=new User();").unwrap();
    let mut foreign = runtime();
    foreign
        .set_global("user", source.get_global("user").unwrap().clone())
        .unwrap();
    assert!(
        run(&mut foreign, "$user->name;")
            .unwrap_err()
            .message
            .contains("different runtime")
    );
    for source in ["class int {}", "class Bad {function f($x,$x) {}}"] {
        assert!(run(&mut runtime(), source).is_err(), "{source}");
    }
}
