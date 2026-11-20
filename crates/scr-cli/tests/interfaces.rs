use scr_runtime::{ContextOptions, Runtime, RuntimeOptions, Value};
use std::fs;

fn run(runtime: &mut Runtime, source: &str) -> Result<(), scr_runtime::RuntimeError> {
    runtime.execute_source(source, &mut Vec::new()).map(|_| ())
}

#[test]
fn interfaces_support_composition_multiple_implementations_and_bound_methods() {
    let mut rt = Runtime::new();
    run(
        &mut rt,
        r#"
        interface Notifier { function send($message: string): string; }
        interface Named { function name(): string; }
        class Email implements Notifier, Named {
            function send($text: string): string { return "Email: " . $text; }
            function name(): string { return "email"; }
        }
        class Sms implements Notifier {
            function send($text: string): string { return "SMS: " . $text; }
        }
        class Service {
            $notifier: Notifier;
            function __construct($notifier: Notifier) { $this->notifier = $notifier; }
            function notify(): string { return $this->notifier->send("hello"); }
            function get(): Notifier { return $this->notifier; }
        }
        function named($value: Named): string { return $value->name(); }
        $email = new Email();
        $service = new Service($email);
        $first = $service->notify();
        $same = $service->get() === $email;
        $name = named($email);
        $method = $service->get()->send;
        $bound = $method("bound");
        $service->notifier = new Sms();
        $second = $service->notify();
        try { $service->notifier = "wrong"; } catch (TypeError $e) {}
        $after = $service->notify();
    "#,
    )
    .unwrap();
    for (name, expected) in [
        ("first", "Email: hello"),
        ("second", "SMS: hello"),
        ("after", "SMS: hello"),
        ("bound", "Email: bound"),
        ("name", "email"),
    ] {
        assert_eq!(rt.get_global(name), Some(&Value::String(expected.into())));
    }
    assert_eq!(rt.get_global("same"), Some(&Value::Bool(true)));
}

#[test]
fn contracts_reject_missing_methods_and_incompatible_signatures() {
    for source in [
        "interface I { function f(): int; } class C implements I {}",
        "interface I { function f(): int; } class C implements I { $f; }",
        "interface I { function f($x: int): int; } class C implements I { function f($x: string): int { return 1; } }",
        "interface I { function f(): int; } class C implements I { function f(): string { return 'x'; } }",
        "interface I { function f($x); } class C implements I { function f() {} }",
        "interface I { function f($x = 1); } class C implements I { function f($x) {} }",
        "interface I { function f(...$x: int); } class C implements I { function f($x: int) {} }",
        "interface I { function f($x: int); } class C implements I { function f($x) {} }",
        "interface I { function f(): void; } interface J { function f(): string; } class C implements I, J { function f(): void {} }",
        "interface I {} class C implements I, I {}",
        "class I {} class C implements I {}",
        "class C implements Missing {}",
        "interface I {} $i = new I();",
    ] {
        let error = run(&mut Runtime::new(), source).unwrap_err();
        assert!(!error.message.is_empty(), "{source}");
    }
}

#[test]
fn interface_types_require_explicit_conformance_and_check_return_values() {
    let mut rt = Runtime::new();
    run(&mut rt, "interface I { function f(): int; } class Similar { function f(): int { return 1; } } function accept($x: I) {} function bad(): I { return new Similar(); }").unwrap();
    for source in ["accept(new Similar());", "$result = bad();"] {
        let error = run(&mut rt, source).unwrap_err();
        assert!(error.message.contains("TypeError"), "{}", error.message);
    }
}

#[test]
fn interface_defaults_are_not_evaluated_and_variadic_contracts_work() {
    let mut rt = Runtime::new();
    run(&mut rt, r#"
        interface I { function f($x: int = missing(), ...$rest: string): int; }
        class C implements I { function f($value: int = 42, ...$tail: string): int { return $value; } }
        function call($x: I): int { return $x->f(); }
        $result = call(new C());
        $more = (new C())->f(7, "a", "b");
    "#).unwrap();
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
    assert_eq!(rt.get_global("more"), Some(&Value::Int(7)));
}

#[test]
fn entry_interfaces_are_hoisted_and_types_share_a_declaration_namespace() {
    run(
        &mut Runtime::new(),
        "function accept($x: I) {} accept(new C()); class C implements I {} interface I {}",
    )
    .unwrap();
    for source in [
        "class I {} interface I {}",
        "interface I {} interface I {}",
        "interface I {} class I {}",
    ] {
        assert!(run(&mut Runtime::new(), source).is_err(), "{source}");
        assert!(scr_cli::pipeline::check(source).is_err(), "{source}");
    }
}

#[test]
fn module_interfaces_and_signature_types_use_lexical_identity() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("contracts.scripta"), "interface I { function reflect($value: User): User; } class User {} class Local implements I { function reflect($x: User): User { return $x; } }").unwrap();
    fs::write(
        dir.path().join("other.scripta"),
        "interface I { function reflect($value: User): User; } class User {}",
    )
    .unwrap();
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: dir.path().into(),
            ..Default::default()
        },
        ..Default::default()
    });
    run(
        &mut rt,
        r#"
        require "contracts.scripta" as Contracts;
        require "other.scripta" as Other;
        class C implements Contracts::I {
            function reflect($user: Contracts::User): Contracts::User { return $user; }
        }
        function accept($value: Contracts::I): Contracts::I { return $value; }
        function other($value: Other::I) {}
        $local = accept(new Contracts::Local());
        $c = accept(new C());
        $user = new Contracts::User();
        $same = $c->reflect($user) === $user;
    "#,
    )
    .unwrap();
    assert_eq!(rt.get_global("same"), Some(&Value::Bool(true)));
    assert!(
        run(&mut rt, "other($c);")
            .unwrap_err()
            .message
            .contains("TypeError")
    );
    let error = run(&mut rt, "class Bad implements Contracts::I { function reflect($x: Other::User): Other::User { return $x; } }").unwrap_err();
    assert!(
        error.message.contains("does not match interface"),
        "{}",
        error.message
    );
    // Required files retain declaration order, unlike the entry script.
    fs::write(
        dir.path().join("bad.scripta"),
        "class TooEarly implements Later {} interface Later {}",
    )
    .unwrap();
    assert!(
        run(&mut rt, "require 'bad.scripta' as Bad;")
            .unwrap_err()
            .message
            .contains("undefined interface")
    );
}

#[test]
fn interface_syntax_formats_idempotently_with_lf_and_crlf() {
    let source = "interface I{function send($x:string):string;}class C implements I{function send($text:string):string{return $text;}}function call($x:I):string{return $x->send('ok');}$result=call(new C());";
    for source in [source.to_owned(), source.replace(';', ";\r\n")] {
        let formatted = scr_cli::formatter::format(&source).unwrap();
        assert!(formatted.contains("function send("), "{formatted}");
        assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
        for source in [&source, &formatted] {
            let mut rt = Runtime::new();
            run(&mut rt, source).unwrap();
            assert_eq!(rt.get_global("result"), Some(&Value::String("ok".into())));
        }
    }
    for source in [
        "function f() { interface I {} }",
        "if (true) { interface I {} }",
        "interface int {}",
        "interface I { $field; }",
        "interface I { function f() {} }",
        "interface I { function f(); function f(); }",
        "interface I { function __construct(); }",
        "interface I { function f($x, $x); }",
        "interface I { function f($x = 1, $y); }",
        "interface I { function f(...$x, $y); }",
        "interface I extends J {}",
        "class C implements {}",
    ] {
        assert!(scr_cli::pipeline::check(source).is_err(), "{source}");
    }
}

#[derive(Debug)]
struct Probe(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
impl scr_runtime::NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Link"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("unknown property".into())
    }
}

#[test]
fn interface_typed_cycles_and_bound_methods_keep_tracing_and_runtime_ownership() {
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
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
    run(
        &mut rt,
        r#"
        interface Link { function get(): Link; }
        class Node implements Link {
            $next: Link;
            $callback;
            $payload;
            function __construct() { $this->payload = probe(); }
            function get(): Link { return $this->next; }
        }
        function accept($value: Link): Link { return $value; }
        for ($i = 0; $i < 100; $i += 1) {
            $node = new Node();
            $node->next = $node;
            $node->callback = $node->get;
            $node = null;
        }
    "#,
    )
    .unwrap();
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 100);
    run(
        &mut rt,
        "$node = new Node(); $node->next = $node; $method = $node->get; $node = null;",
    )
    .unwrap();
    let method = rt.get_global("method").unwrap().clone();
    run(&mut rt, "$method = null;").unwrap();
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 100);
    let node = rt.invoke(method.clone(), &[], &mut Vec::new()).unwrap();
    let mut foreign = Runtime::new();
    run(
        &mut foreign,
        "interface Link {} function accept($value: Link) {}",
    )
    .unwrap();
    foreign.set_global("foreign", node.clone()).unwrap();
    assert!(
        run(&mut foreign, "accept($foreign);")
            .unwrap_err()
            .message
            .contains("TypeError")
    );
    drop(foreign);
    drop(node);
    drop(method);
    rt.collect_garbage();
    assert_eq!(dropped.load(Ordering::SeqCst), 101);
    // A native object cannot acquire conformance by returning an interface name.
    assert!(
        run(&mut rt, "accept(probe());")
            .unwrap_err()
            .message
            .contains("TypeError")
    );
}
