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
                Value::map(vec![
                    (MapKey::String("id".into()), Value::Int(i)),
                    (MapKey::String("v".into()), Value::Int(i)),
                ])
            })
            .collect();
        for (name, value) in [
            ("left", Value::map(left)),
            ("right", Value::map(right)),
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
    callback_timings();
}

fn callback_timings() {
    for size in [128_i64, 512, 2048] {
        let mut runtime = Runtime::new();
        scr_stdlib::register(&mut runtime).unwrap();
        let values = Value::List((0..size).map(Value::Int).collect());
        let ranks = Value::List((0..size).rev().map(Value::Int).collect());
        let rows: Vec<_> = (0..size)
            .map(|index| {
                Value::map(vec![
                    (MapKey::String("rank".into()), Value::Int(size - 1 - index)),
                    (
                        MapKey::String("payload".into()),
                        Value::list(vec![
                            Value::String("x".repeat(128)),
                            Value::String("世界".into()),
                        ]),
                    ),
                ])
            })
            .collect();
        let sorted = Value::List(rows.iter().rev().cloned().collect());
        runtime.set_global("values", values.clone()).unwrap();
        runtime.set_global("rows", Value::list(rows)).unwrap();
        runtime.execute_source(r#"
            function square($value:int):int {return $value*$value;}
            function even($value:int):bool {return $value%2===0;}
            function add($acc:int,$value:int):int {return $acc+$value;}
            function rank($row):int {return $row["rank"];}
            function compareRank($left,$right):int {if($left["rank"]<$right["rank"])return -1;if($left["rank"]>$right["rank"])return 1;return 0;}
            function appendValue($acc,$value) {$acc[]=$value;return $acc;}
        "#,&mut std::io::sink()).unwrap();
        let squares = Value::List((0..size).map(|v| Value::Int(v * v)).collect());
        let filtered = Value::List((0..size).filter(|v| v % 2 == 0).map(Value::Int).collect());
        let sum = Value::Int(size * (size - 1) / 2);
        let cases = [
            (
                "callback_map_scalar",
                "$result=List::map($values,square);",
                squares.clone(),
            ),
            (
                "comprehension_map_scalar",
                "$result=[$v*$v for $v in $values];",
                squares.clone(),
            ),
            (
                "foreach_map_scalar",
                "$result=[];foreach($values as $v){$result[]=$v*$v;}",
                squares,
            ),
            (
                "callback_filter_scalar",
                "$result=List::filter($values,even);",
                filtered.clone(),
            ),
            (
                "comprehension_filter_scalar",
                "$result=[$v for $v in $values if $v%2===0];",
                filtered,
            ),
            (
                "callback_reduce_scalar",
                "$result=List::reduce($values,add,0);",
                sum.clone(),
            ),
            (
                "fixed_sum_scalar",
                "$result=List::sum($values);",
                sum.clone(),
            ),
            (
                "foreach_sum_scalar",
                "$result=0;foreach($values as $v){$result+=$v;}",
                sum,
            ),
            (
                "callback_map_record",
                "$result=List::map($rows,rank);",
                ranks.clone(),
            ),
            (
                "comprehension_map_record",
                r#"$result=[$row["rank"] for $row in $rows];"#,
                ranks.clone(),
            ),
            (
                "fixed_column_record",
                r#"$result=List::column($rows,"rank");"#,
                ranks,
            ),
            (
                "callback_sort_key_record",
                "$result=List::sortBy($rows,rank);",
                sorted.clone(),
            ),
            (
                "callback_sort_compare_record",
                "$result=List::sortWith($rows,compareRank);",
                sorted,
            ),
            (
                "callback_reduce_growing_list",
                "$result=List::reduce($values,appendValue,[]);",
                values.clone(),
            ),
            (
                "foreach_build_list",
                "$result=[];foreach($values as $v){$result[]=$v;}",
                values,
            ),
        ];
        let programs: Vec<_> = cases
            .iter()
            .map(|(_, source, _)| scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap())
            .collect();
        let mut samples = vec![Vec::new(); cases.len()];
        // Reverse traversal every other sample to reduce fixed-order bias.
        for iteration in 0..11 {
            for position in 0..cases.len() {
                let index = if iteration % 2 == 0 {
                    position
                } else {
                    cases.len() - 1 - position
                };
                let start = Instant::now();
                runtime
                    .execute(black_box(&programs[index]), &mut std::io::sink())
                    .unwrap();
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(
                    runtime.get_global("result"),
                    Some(&cases[index].2),
                    "{}",
                    cases[index].0
                );
                black_box(runtime.get_global("result").unwrap());
                if iteration >= 2 {
                    samples[index].push(elapsed);
                }
            }
        }
        for ((name, _, _), mut times) in cases.into_iter().zip(samples) {
            times.sort_by(f64::total_cmp);
            println!(
                "{name},{size},{},{:.6}",
                times.len(),
                times[times.len() / 2]
            );
        }
    }
}
