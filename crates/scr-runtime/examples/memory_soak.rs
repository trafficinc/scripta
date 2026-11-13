//! Checkpoint worker for benchmarks/memory_soak.py; run through that driver.
use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, Value};
use std::io::{self, Write};

fn parse(body: &str) -> scr_ast::Program {
    scr_parser::parse(&scr_lexer::lex(&format!("<?scr {body}")).unwrap()).unwrap()
}

fn runtime(setup: &scr_ast::Program) -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .register(
            "payload",
            NativeFunction::new(NativeSignature::new(vec![], NativeType::String), |_, _| {
                Ok(Value::String("x".repeat(4096)))
            }),
        )
        .unwrap();
    runtime.execute(setup, &mut io::sink()).unwrap();
    runtime
}

fn checkpoint(phase: &str, batch: usize) {
    println!("{phase} {batch}");
    io::stdout().flush().unwrap();
    let mut ack = String::new();
    assert!(
        io::stdin().read_line(&mut ack).unwrap() > 0,
        "driver disconnected"
    );
    assert_eq!(ack.trim(), "next");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(
        args.len(),
        5,
        "usage: memory_soak WORKLOAD WARMUPS BATCHES ITERATIONS"
    );
    let workload = args[1].as_str();
    let warmups: usize = args[2].parse().unwrap();
    let batches: usize = args[3].parse().unwrap();
    let iterations: usize = args[4].parse().unwrap();
    assert!(batches > 0 && iterations > 0);
    let (definition, call) = match workload {
        "transient" | "recreate" => (
            r#"function work() {
                $text=payload(); $list=[$text, $text];
                $map=["items" => $list]; $copy=$map;
                $copy["items"][0]="changed";
            }"#,
            "work();",
        ),
        "errors" => (
            r#"function work($fail) {
                $text=payload(); $items=[$text];
                if ($fail) { throw "expected"; }
                1 / 0;
            }"#,
            "try { work(true); } catch ($e) {} try { work(false); } catch (ArithmeticError $e) {}",
        ),
        "closures" => (
            "function factory() { $text=payload(); return function () { return $text; }; }",
            "$f=factory(); $f=null;",
        ),
        _ => panic!("unknown workload"),
    };
    let setup = parse(&format!("{definition} $i=0; $f=null;"));
    let program = parse(&format!("for ($i=0; $i<{iterations}; $i+=1) {{ {call} }}"));
    let mut active = runtime(&setup);
    for batch in 0..warmups + batches {
        if workload == "recreate" {
            drop(active);
            active = runtime(&setup);
        }
        active.execute(&program, &mut io::sink()).unwrap();
        let expected = if workload == "closures" {
            Value::Null
        } else {
            Value::Int(iterations as i64)
        };
        let name = if workload == "closures" { "f" } else { "i" };
        assert_eq!(active.get_global(name), Some(&expected));
        if batch + 1 == warmups {
            checkpoint("baseline", 0);
        }
        if batch >= warmups {
            checkpoint("sample", batch + 1 - warmups);
        }
    }
    drop(active);
    checkpoint("dropped", batches);
}
