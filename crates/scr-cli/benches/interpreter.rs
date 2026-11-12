use std::{hint::black_box, time::Instant};
fn main() {
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/fixtures");
    for name in [
        "startup",
        "variables",
        "calls",
        "loops",
        "strings",
        "lists",
        "maps",
    ] {
        let source = std::fs::read_to_string(fixtures.join(format!("{name}.scr"))).unwrap();
        let tokens = scr_lexer::lex(&source).unwrap();
        let program = scr_parser::parse(&tokens).unwrap();
        for phase in ["lex", "parse", "execute"] {
            let iterations = if phase == "execute" { 9 } else { 1000 };
            let mut observations = Vec::new();
            for _ in 0..iterations {
                let mut runtime = scr_runtime::Runtime::new();
                if phase == "execute" {
                    scr_stdlib::register(&mut runtime).unwrap();
                }
                let start = Instant::now();
                match phase {
                    "lex" => {
                        black_box(scr_lexer::lex(black_box(&source)).unwrap());
                    }
                    "parse" => {
                        black_box(scr_parser::parse(black_box(&tokens)).unwrap());
                    }
                    _ => {
                        runtime
                            .execute(black_box(&program), &mut std::io::sink())
                            .unwrap();
                    }
                }
                observations.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            observations.sort_by(f64::total_cmp);
            println!(
                "{name},{phase},{iterations},{:.6}",
                observations[iterations / 2]
            );
        }
    }
    let values = vec![scr_runtime::Value::Int(7); 4096];
    for mode in ["owned_clone", "rc_clone", "cow_write"] {
        let shared = std::rc::Rc::new(values.clone());
        let start = Instant::now();
        for _ in 0..10000 {
            match mode {
                "owned_clone" => {
                    black_box(values.clone());
                }
                "rc_clone" => {
                    black_box(shared.clone());
                }
                _ => {
                    let mut snapshot = shared.clone();
                    std::rc::Rc::make_mut(&mut snapshot)[0] = scr_runtime::Value::Int(8);
                    black_box(snapshot);
                }
            }
        }
        println!(
            "sharing,{mode},10000,{:.6}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
