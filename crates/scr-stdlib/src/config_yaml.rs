use super::*;
use regex::Regex;
use std::sync::OnceLock;
use yaml_rust2::{
    parser::{Event, Parser},
    scanner::{Marker, TScalarStyle},
};

fn at(mark: Marker, message: &str) -> NativeError {
    located("Yaml", mark.line(), mark.col() + 1, message)
}
fn scalar(text: String, style: TScalarStyle, mark: Marker) -> NativeResult<Value> {
    if style != TScalarStyle::Plain {
        return Ok(Value::String(text));
    }
    match text.as_str() {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Value::Null),
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
        | ".nan" | ".NaN" | ".NAN" => return Err(at(mark, "nonfinite floats are unsupported")),
        _ => {}
    }
    let decimal = text.strip_prefix(['+', '-']).unwrap_or(&text);
    if !decimal.is_empty() && decimal.bytes().all(|b| b.is_ascii_digit()) {
        return text
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| at(mark, "integer is outside i64 range"));
    }
    for (prefix, radix) in [("0o", 8), ("0x", 16)] {
        if let Some(digits) = text
            .strip_prefix(prefix)
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_digit(radix)))
        {
            return i64::from_str_radix(digits, radix)
                .map(Value::Int)
                .map_err(|_| at(mark, "integer is outside i64 range"));
        }
    }
    static FLOAT: OnceLock<Regex> = OnceLock::new();
    let float = FLOAT.get_or_init(|| {
        Regex::new(r"^[-+]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][-+]?[0-9]+)?$").unwrap()
    });
    if float.is_match(&text) {
        let number = text.parse::<f64>().map_err(|_| at(mark, "invalid float"))?;
        if !number.is_finite() {
            return Err(at(mark, "float is outside finite f64 range"));
        }
        return Ok(Value::Float(number));
    }
    Ok(Value::String(text))
}
fn next(
    parser: &mut Parser<std::str::Chars<'_>>,
    budget: &Budget<'_, '_>,
) -> NativeResult<(Event, Marker)> {
    budget.context.check_cancelled()?;
    let event = parser.next_token();
    budget.context.check_cancelled()?;
    event.map_err(|e| at(*e.marker(), "invalid YAML"))
}
fn key(event: Event, mark: Marker) -> NativeResult<String> {
    let Event::Scalar(text, style, 0, None) = event else {
        return Err(at(
            mark,
            "mapping keys must be untagged strings without anchors",
        ));
    };
    let Value::String(text) = scalar(text, style, mark)? else {
        return Err(at(
            mark,
            "mapping keys must resolve to strings; quote typed keys",
        ));
    };
    if text == "<<" {
        return Err(at(mark, "merge keys are unsupported"));
    }
    Ok(text)
}
fn node(
    parser: &mut Parser<std::str::Chars<'_>>,
    budget: &mut Budget<'_, '_>,
    event: Event,
    mark: Marker,
    depth: usize,
) -> NativeResult<Value> {
    // Bound recursion before requesting any child event or constructing its value.
    budget.value(depth, 0)?;
    match event {
        Event::Scalar(text, style, 0, None) => {
            let value = scalar(text, style, mark)?;
            if let Value::String(text) = &value {
                budget.add(text.len())?;
            }
            Ok(value)
        }
        Event::SequenceStart(0, None) => {
            let mut items = Vec::new();
            loop {
                let (event, mark) = next(parser, budget)?;
                if event == Event::SequenceEnd {
                    break;
                }
                let value = node(parser, budget, event, mark, depth + 1)?;
                items.push(value);
            }
            Ok(Value::list(items))
        }
        Event::MappingStart(0, None) => {
            let mut items = Vec::new();
            let mut names = BTreeSet::new();
            loop {
                let (event, mark) = next(parser, budget)?;
                if event == Event::MappingEnd {
                    break;
                }
                let name = key(event, mark)?;
                budget.key(&name)?;
                if !names.insert(name.clone()) {
                    return Err(at(mark, "duplicate mapping key"));
                }
                let (event, mark) = next(parser, budget)?;
                let value = node(parser, budget, event, mark, depth + 1)?;
                items.push((MapKey::String(name.into()), value));
            }
            Ok(Value::map(items))
        }
        Event::Alias(_) => Err(at(mark, "aliases are unsupported")),
        Event::Scalar(..) | Event::SequenceStart(..) | Event::MappingStart(..) => {
            Err(at(mark, "anchors and explicit tags are unsupported"))
        }
        _ => Err(at(mark, "unexpected YAML event")),
    }
}
pub(super) fn parse(budget: &mut Budget<'_, '_>, text: &str) -> NativeResult<Value> {
    let mut parser = Parser::new(text.chars());
    let (event, mark) = next(&mut parser, budget)?;
    if event != Event::StreamStart {
        return Err(at(mark, "expected YAML stream"));
    }
    let (event, mark) = next(&mut parser, budget)?;
    if event == Event::StreamEnd {
        budget.value(0, 0)?;
        return Ok(Value::Null);
    }
    if event != Event::DocumentStart {
        return Err(at(mark, "expected YAML document"));
    }
    let (event, mark) = next(&mut parser, budget)?;
    let result = node(&mut parser, budget, event, mark, 0)?;
    let (event, mark) = next(&mut parser, budget)?;
    if event != Event::DocumentEnd {
        return Err(at(mark, "expected YAML document end"));
    }
    let (event, mark) = next(&mut parser, budget)?;
    if event != Event::StreamEnd {
        return Err(at(mark, "multiple YAML documents are unsupported"));
    }
    Ok(result)
}
