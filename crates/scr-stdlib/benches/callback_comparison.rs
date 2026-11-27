//! Paired with benchmarks/callbacks/node.cjs; see that directory's README.
use scr_runtime::{MapKey, Runtime, Value};
use serde_json::{Value as Json, json};
use std::{hint::black_box, time::Instant};

fn value(input: &Json) -> Value {
    match input {
        Json::Number(n) => Value::Int(n.as_i64().unwrap()),
        Json::String(s) => Value::String(s.clone()),
        Json::Array(items) => Value::List(items.iter().map(value).collect()),
        Json::Object(items) => Value::Map(
            items
                .iter()
                .map(|(k, v)| (MapKey::String(k.as_str().into()), value(v)))
                .collect(),
        ),
        _ => panic!("unsupported fixture value"),
    }
}
fn output(input: &Value) -> Json {
    match input {
        Value::Int(n) => json!(n),
        Value::String(s) => json!(s),
        Value::List(items) => Json::Array(items.iter().map(output).collect()),
        Value::Map(items) => Json::Array(
            items
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        MapKey::Int(n) => json!(n),
                        MapKey::String(s) => json!(s.as_str()),
                    };
                    json!([key, output(v)])
                })
                .collect(),
        ),
        _ => panic!("unsupported benchmark result"),
    }
}
fn main() {
    let Some(path) = std::env::args().nth(1).filter(|arg| arg != "--test") else {
        eprintln!("Supply a fixture JSON path; see benchmarks/callbacks/README.md");
        return;
    };
    let fixture: Json = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut results = Vec::new();
    for dataset in fixture["datasets"].as_array().unwrap() {
        let mut runtime = Runtime::new();
        scr_stdlib::register(&mut runtime).unwrap();
        runtime
            .set_global("values", value(&dataset["values"]))
            .unwrap();
        runtime.set_global("rows", value(&dataset["rows"])).unwrap();
        let entries = dataset["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| {
                let key = if let Some(n) = pair[0].as_i64() {
                    MapKey::Int(n)
                } else {
                    MapKey::String(pair[0].as_str().unwrap().into())
                };
                (key, value(&pair[1]))
            })
            .collect();
        runtime.set_global("entries", Value::Map(entries)).unwrap();
        runtime.execute_source(r#"
            function square($v:int):int {return $v*$v;}
            function makeOffset($amount) {return function($v) {return $v+$amount;};}
            $captured=makeOffset(7);
            function even($v:int):bool {return $v%2===0;}
            function add($a:int,$v:int):int {return $a+$v;}
            function rank($v):int {return $v["rank"];}
            function compareRank($a,$b):int {if($a["rank"]<$b["rank"])return -1;if($a["rank"]>$b["rank"])return 1;return 0;}
            function mapValue($v:int,$key):int {return $v+1;}
            function filterValue($v:int,$key):bool {return $v%2===0;}
            function reduceValue($a:int,$v:int,$key):int {return $a+$v;}
            function append($a,$v) {$a[]=$v;return $a;}
        "#, &mut std::io::sink()).unwrap();
        let cases = [
            ("list_map", "List::map($values,square)"),
            ("list_map_capture", "List::map($values,$captured)"),
            ("list_filter", "List::filter($values,even)"),
            ("list_reduce", "List::reduce($values,add,0)"),
            ("map_map", "Map::map($entries,mapValue)"),
            ("map_filter", "Map::filter($entries,filterValue)"),
            ("map_reduce", "Map::reduce($entries,reduceValue,0)"),
            ("record_map", "List::map($rows,rank)"),
            ("record_sort_key", "List::sortBy($rows,rank)"),
            ("record_sort_compare", "List::sortWith($rows,compareRank)"),
            ("reduce_growing_copy", "List::reduce($values,append,[])"),
        ];
        let programs: Vec<_> = cases
            .iter()
            .map(|(_, expression)| {
                scr_parser::parse(&scr_lexer::lex(&format!("$result={expression};")).unwrap())
                    .unwrap()
            })
            .collect();
        let mut samples = vec![Vec::new(); cases.len()];
        let mut repeats = Vec::new();
        for program in &programs {
            let start = Instant::now();
            let mut count = 0_usize;
            while count == 0 || start.elapsed().as_secs_f64() < 0.1 {
                runtime
                    .execute(black_box(program), &mut std::io::sink())
                    .unwrap();
                count += 1;
            }
            let per_call = start.elapsed().as_secs_f64() / count as f64;
            repeats.push((0.01 / per_call).ceil().clamp(1.0, 10000.0) as usize);
        }
        for sample in 0..11 {
            for position in 0..cases.len() {
                let i = if sample % 2 == 0 {
                    position
                } else {
                    cases.len() - 1 - position
                };
                let start = Instant::now();
                for _ in 0..repeats[i] {
                    runtime
                        .execute(black_box(&programs[i]), &mut std::io::sink())
                        .unwrap();
                }
                let ms = start.elapsed().as_secs_f64() * 1000.0 / repeats[i] as f64;
                assert_eq!(
                    output(runtime.get_global("result").unwrap()),
                    dataset["expected"][cases[i].0],
                    "{}",
                    cases[i].0
                );
                samples[i].push(ms);
            }
        }
        for (i, (name, _)) in cases.iter().enumerate() {
            results.push(json!({"operation":name,"size":dataset["size"],"batch":repeats[i],"samples_ms":samples[i]}));
        }
    }
    println!("{}", json!({"runtime":"scripta","results":results}));
}
