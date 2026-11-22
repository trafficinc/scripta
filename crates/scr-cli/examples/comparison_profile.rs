//! Checked execution-only worker; CLI timing remains in benchmarks/comparison/run.py.
use scr_runtime::{ExecutionOutcome, Runtime, RuntimeOptions};
use serde::Serialize;
use std::{
    hint::black_box,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Default, Serialize)]
struct Observations {
    lex_ms: Vec<f64>,
    parse_ms: Vec<f64>,
    setup_ms: Vec<f64>,
    execute_ms: Vec<f64>,
    teardown_ms: Vec<f64>,
}

fn runtime(path: &Path) -> Runtime {
    let mut options = RuntimeOptions::default();
    options.context.script_path = Some(path.to_path_buf());
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).expect("standard-library registration");
    #[cfg(feature = "sqlite")]
    scr_ext_sqlite::register(&mut runtime).expect("SQLite registration");
    runtime
}

fn execute(runtime: &mut Runtime, program: &scr_ast::Program, expected: &[u8]) {
    let mut output = Vec::new();
    let outcome = runtime
        .execute_outcome(black_box(program), &mut output)
        .expect("execution");
    assert_eq!(
        outcome,
        ExecutionOutcome::Completed,
        "unexpected script exit"
    );
    assert_eq!(output, expected, "fixture output mismatch");
}

fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    assert!(
        !black_box(cfg!(debug_assertions)),
        "use an optimized worker"
    );
    let args: Vec<_> = std::env::args().collect();
    assert!(
        (5..=7).contains(&args.len()),
        "usage: comparison_profile FIXTURE EXPECTED SAMPLES WARMUPS [measure|cpu|alloc] [SECONDS]"
    );
    let path = Path::new(&args[1]).canonicalize().expect("fixture path");
    let source = std::fs::read_to_string(&path).expect("fixture source");
    let expected = std::fs::read(&args[2]).expect("expected output");
    let samples: usize = args[3].parse().expect("samples");
    let warmups: usize = args[4].parse().expect("warmups");
    assert!(samples > 0 && samples <= 10_000 && warmups <= 10_000);
    let mode = args.get(5).map_or("measure", String::as_str);
    let seconds: u64 = args.get(6).map_or(5, |s| s.parse().expect("seconds"));
    assert!((1..=300).contains(&seconds));
    scr_cli::pipeline::check(&source).expect("fixture validation");
    let tokens = scr_lexer::lex(&source).expect("lex");
    let program = scr_parser::parse(&tokens).expect("parse");
    if mode == "cpu" {
        // Setup/teardown are present in sampled stacks but outside execute_ms.
        eprintln!("READY");
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            execute(&mut runtime(&path), &program, &expected);
            iterations += 1;
        }
        println!(
            "{}",
            serde_json::json!({"mode": mode, "iterations": iterations})
        );
        return;
    }
    if mode == "alloc" {
        // Keep results alive so native tools can inspect live and peak allocation stacks.
        let mut runtime = runtime(&path);
        execute(&mut runtime, &program, &expected);
        eprintln!("READY");
        std::thread::sleep(Duration::from_secs(seconds));
        black_box(&runtime);
        println!("{}", serde_json::json!({"mode": mode, "iterations": 1}));
        return;
    }
    assert_eq!(mode, "measure", "unknown mode");
    let mut observations = Observations::default();
    for index in 0..warmups + samples {
        let start = Instant::now();
        let lexed = scr_lexer::lex(black_box(&source)).expect("lex");
        let lex_ms = elapsed(start);
        let start = Instant::now();
        let parsed = scr_parser::parse(black_box(&tokens)).expect("parse");
        let parse_ms = elapsed(start);
        let start = Instant::now();
        let mut runtime = runtime(&path);
        let setup_ms = elapsed(start);
        let mut output = Vec::new();
        let start = Instant::now();
        let outcome = runtime
            .execute_outcome(black_box(&program), &mut output)
            .expect("execution");
        let execute_ms = elapsed(start);
        assert_eq!(
            outcome,
            ExecutionOutcome::Completed,
            "unexpected script exit"
        );
        assert_eq!(output, expected, "fixture output mismatch");
        let start = Instant::now();
        drop(runtime);
        let teardown_ms = elapsed(start);
        if index >= warmups {
            observations.lex_ms.push(lex_ms);
            observations.parse_ms.push(parse_ms);
            observations.setup_ms.push(setup_ms);
            observations.execute_ms.push(execute_ms);
            observations.teardown_ms.push(teardown_ms);
        }
        black_box((lexed, parsed));
    }
    println!(
        "{}",
        serde_json::to_string(&observations).expect("JSON report")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> scr_ast::Program {
        scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap()
    }

    #[test]
    fn samples_use_independent_runtime_globals() {
        let program = parse("$value = 1; echo str($value); $value = 9;");
        for _ in 0..2 {
            let mut runtime = runtime(Path::new("fixture.scripta"));
            assert!(runtime.get_global("value").is_none());
            execute(&mut runtime, &program, b"1");
            assert_eq!(
                runtime.get_global("value"),
                Some(&scr_runtime::Value::Int(9))
            );
        }
    }

    #[test]
    #[should_panic(expected = "fixture output mismatch")]
    fn wrong_output_is_rejected() {
        execute(
            &mut runtime(Path::new("fixture.scripta")),
            &parse("echo str(42);"),
            b"41",
        );
    }

    #[test]
    #[should_panic(expected = "unexpected script exit")]
    fn explicit_exit_zero_does_not_silently_skip_work() {
        execute(
            &mut runtime(Path::new("fixture.scripta")),
            &parse("exit(0);"),
            b"",
        );
    }
}
