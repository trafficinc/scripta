//! Isolated optimized benchmark worker driven by benchmarks/collector.py.
use scr_runtime::{Runtime, Value};
use std::{hint::black_box, io, time::Instant};

fn parse(body: &str) -> scr_ast::Program {
    scr_parser::parse(&scr_lexer::lex(&format!("<?scripta {body}")).unwrap()).unwrap()
}

fn execute(runtime: &mut Runtime, program: &scr_ast::Program) {
    assert_eq!(runtime.execute(program, &mut io::sink()).unwrap(), 0);
}

struct Fixture {
    runtime: Runtime,
    host: Option<Value>,
    retained: usize,
    expected_reclaimed: usize,
}

fn prepare(case: &str, size: usize, setup: &scr_ast::Program) -> Fixture {
    let mut runtime = Runtime::new();
    execute(&mut runtime, setup);
    let mut kept = runtime.get_global("kept").unwrap().clone();
    let retained = match case {
        "discarded" | "recursive" => 0,
        "mixed" => size * 9 / 10,
        _ => size,
    };
    let Value::List(values) = &mut kept else {
        unreachable!()
    };
    assert_eq!(values.len(), size);
    values.truncate(retained);
    runtime.set_global("kept", Value::Null).unwrap();
    let host = if case == "host" {
        Some(kept)
    } else {
        runtime.set_global("kept", kept).unwrap();
        None
    };
    Fixture {
        runtime,
        host,
        retained,
        expected_reclaimed: if case == "plain_values" {
            0
        } else {
            2 * (size - retained)
        },
    }
}

fn verify(case: &str, fixture: &mut Fixture) {
    if let Some(host) = fixture.host.take() {
        fixture.runtime.set_global("kept", host).unwrap();
    }
    if fixture.retained > 0 && case != "plain_values" {
        let call = match case {
            "shared" => "$answer=$kept[0][0](); $shared=$kept[0][1]();",
            "nested" => "$answer=$kept[0][\"wrapped\"][0][0]();",
            _ => "$answer=$kept[0]();",
        };
        execute(&mut fixture.runtime, &parse(call));
        let expected = if case == "shared" { 1 } else { 0 };
        assert_eq!(
            fixture.runtime.get_global("answer"),
            Some(&Value::Int(expected))
        );
        if case == "shared" {
            assert_eq!(fixture.runtime.get_global("shared"), Some(&Value::Int(1)));
        }
    }
    assert_eq!(
        fixture.runtime.collect_garbage(),
        0,
        "second collection must be a no-op"
    );
    fixture.runtime.set_global("kept", Value::Null).unwrap();
    let remaining = if case == "plain_values" {
        0
    } else {
        2 * fixture.retained
    };
    assert_eq!(
        fixture.runtime.collect_garbage(),
        remaining,
        "live captures must become reclaimable"
    );
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        5,
        "usage: collector_bench CASE SIZE SAMPLES WARMUPS"
    );
    let case = args[1].as_str();
    let size: usize = args[2].parse().unwrap();
    let samples: usize = args[3].parse().unwrap();
    let warmups: usize = args[4].parse().unwrap();
    assert!(size > 0 && samples > 0);
    let factory = match case {
        "recursive" => {
            "function factory($n) { $f=function ($depth=0) { if ($depth===0) return $n; return $f($depth-1); }; return $f; }"
        }
        "shared" => {
            "function factory($n) { return [function () { $n+=1; return $n; }, function () { return $n; }]; }"
        }
        _ => "function factory($n) { return function () { return $n; }; }",
    };
    let execution_case = matches!(case, "ordinary" | "closure_churn");
    let (setup, batch) = if execution_case {
        let setup = parse(&format!("{factory} $i=0; $f=null;"));
        let body = if case == "ordinary" {
            "$text=\"a\" . \"b\"; $items=[$i, $text]; $sum+=1;"
        } else {
            "$f=factory($i); $f=null; $sum+=1;"
        };
        (
            setup,
            Some(parse(&format!(
                "$sum=0; for ($i=0; $i<{size}; $i+=1) {{ {body} }}"
            ))),
        )
    } else {
        assert!(matches!(
            case,
            "discarded"
                | "live"
                | "mixed"
                | "recursive"
                | "shared"
                | "nested"
                | "host"
                | "plain_values"
        ));
        let item = match case {
            "nested" => "[\"wrapped\" => [[factory($i)]]]",
            "plain_values" => "$i",
            _ => "factory($i)",
        };
        (
            parse(&format!(
                "{factory} $kept=[]; for ($i=0; $i<{size}; $i+=1) {{ $kept[]={item}; }}"
            )),
            None,
        )
    };
    let mut observations = Vec::new();
    let mut reclaimed_samples = Vec::new();
    for observation in 0..warmups + samples {
        let (elapsed, reclaimed) = if let Some(batch) = &batch {
            let mut runtime = Runtime::new();
            execute(&mut runtime, &setup);
            let started = Instant::now();
            execute(&mut runtime, black_box(batch));
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(runtime.get_global("sum"), Some(&Value::Int(size as i64)));
            assert_eq!(runtime.collect_garbage(), 0);
            (elapsed, 0)
        } else {
            let mut fixture = prepare(case, size, &setup);
            let started = Instant::now();
            let reclaimed = black_box(fixture.runtime.collect_garbage());
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(reclaimed, fixture.expected_reclaimed);
            verify(case, &mut fixture);
            (elapsed, reclaimed)
        };
        if observation >= warmups {
            observations.push(elapsed);
            reclaimed_samples.push(reclaimed);
        }
    }
    let phase = if execution_case { "execute" } else { "collect" };
    println!(
        "{{\"case\":\"{case}\",\"size\":{size},\"phase\":\"{phase}\",\"debug_assertions\":{},\"observations_ns\":{:?},\"reclaimed_scopes\":{:?}}}",
        cfg!(debug_assertions),
        observations,
        reclaimed_samples,
    );
}
