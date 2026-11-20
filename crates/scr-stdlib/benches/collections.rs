//! End-to-end helper timings: parsing, registration, and fixture construction
//! are outside the timer; argument copies, runtime checks, and result assignment
//! are included. Timings are observations, never pass/fail thresholds.
use scr_runtime::{MapKey, Runtime, Value};
use std::{hint::black_box, time::Instant};

fn key(index: i64) -> MapKey {
    if index % 2 == 0 {
        MapKey::Int(index)
    } else {
        MapKey::String(index.to_string())
    }
}

fn main() {
    println!("operation,entries,samples,median_ms");
    for size in [128, 512, 2048] {
        let mut runtime = Runtime::new();
        scr_stdlib::register(&mut runtime).unwrap();
        let left: Vec<_> = (0..size).map(|i| (key(i), Value::Int(i))).collect();
        let right: Vec<_> = (size..2 * size).map(|i| (key(i), Value::Int(i))).collect();
        let keys = left
            .iter()
            .map(|(key, _)| match key {
                MapKey::Int(i) => Value::Int(*i),
                MapKey::String(s) => Value::String(s.clone()),
            })
            .collect();
        let values = (0..size).map(Value::Int).collect();
        let rows = (0..size)
            .map(|i| {
                Value::Map(vec![
                    (MapKey::String("id".into()), Value::Int(i)),
                    (MapKey::String("v".into()), Value::Int(i)),
                ])
            })
            .collect();
        for (name, value) in [
            ("left", Value::Map(left)),
            ("right", Value::Map(right)),
            ("keys", Value::List(keys)),
            ("values", Value::List(values)),
            ("rows", Value::List(rows)),
        ] {
            runtime.set_global(name, value).unwrap();
        }
        for (name, expression, expected_len) in [
            ("merge_disjoint", "Map::merge($left, $right)", 2 * size),
            ("merge_overwrite", "Map::merge($left, $left)", size),
            ("replace_disjoint", "Map::replace($left, $right)", size),
            ("combine", "Map::combine($keys, $values)", size),
            ("flip", "Map::flip($left)", size),
            ("column", "Map::column($rows, 'v', 'id')", size),
            ("diff_keys", "Map::diffKeys($left, $right)", size),
            ("intersect_keys", "Map::intersectKeys($left, $left)", size),
            ("diff_values", "Map::diff($left, $right)", size),
        ] {
            let source = format!("<?scr $result = {expression};");
            let tokens = scr_lexer::lex(&source).unwrap();
            let program = scr_parser::parse(&tokens).unwrap();
            let mut samples = Vec::new();
            for iteration in 0..11 {
                let start = Instant::now();
                runtime
                    .execute(black_box(&program), &mut std::io::sink())
                    .unwrap();
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                let Value::Map(result) = runtime.get_global("result").unwrap() else {
                    panic!("map result")
                };
                assert_eq!(
                    result.len(),
                    usize::try_from(expected_len).unwrap(),
                    "{name}"
                );
                black_box(result);
                if iteration >= 2 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{name},{size},{},{:.6}",
                samples.len(),
                samples[samples.len() / 2]
            );
        }
    }
}
