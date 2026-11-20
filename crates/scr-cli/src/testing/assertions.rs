use super::*;
use scr_runtime::{
    InvocationContext, InvocationResult, NativeError, NativeFunction, NativeSignature, NativeType,
    Runtime, RuntimeError, Value,
};
use std::{
    cell::RefCell,
    fmt::{self, Write},
    rc::Rc,
};

#[derive(Default)]
pub(super) struct State {
    pub failure: Option<RuntimeError>,
    pub skipped: Option<String>,
}
pub(super) type SharedState = Rc<RefCell<State>>;

struct LimitedText(String);
impl fmt::Write for LimitedText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = 4096_usize.saturating_sub(self.0.len());
        let mut end = s.len().min(remaining);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&s[..end]);
        if end < s.len() {
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}
fn describe(value: &Value) -> String {
    let mut text = LimitedText(String::new());
    let _ = write!(text, "{value:?}");
    text.0
}
fn failure(
    context: &InvocationContext<'_>,
    state: &SharedState,
    message: String,
) -> InvocationResult {
    let error = context.error(NativeError::new("AssertionError", message));
    let mut state = state.borrow_mut();
    if state.failure.is_none() {
        state.failure = error.diagnostic().cloned();
    }
    Err(error)
}
fn with_message(message: String, values: &[Value], index: usize) -> String {
    if let Some(Value::String(custom)) = values.get(index) {
        format!("{}\n{}", clipped(custom, 4096), message)
    } else {
        message
    }
}

pub(super) fn register(
    runtime: &mut Runtime,
    state: SharedState,
    temp_dir: PathBuf,
) -> Result<(), String> {
    for (name, parameters) in [
        ("same", vec![NativeType::Any, NativeType::Any]),
        ("notSame", vec![NativeType::Any, NativeType::Any]),
        ("isTrue", vec![NativeType::Any]),
        ("isFalse", vec![NativeType::Any]),
        ("contains", vec![NativeType::Any, NativeType::Any]),
    ] {
        let state = state.clone();
        let message_index = parameters.len();
        runtime.register(format!("Assert::{name}"), NativeFunction::reentrant(
            NativeSignature::new(parameters, NativeType::Null).with_optional(vec![NativeType::String]),
            move |context, values| {
                let passed = match name {
                    "same" => values[0] == values[1], "notSame" => values[0] != values[1],
                    "isTrue" => values[0] == Value::Bool(true), "isFalse" => values[0] == Value::Bool(false),
                    "contains" => match (&values[0], &values[1]) {
                        (Value::String(needle), Value::String(value)) => value.contains(needle),
                        (needle, Value::List(values)) => {
                            let mut found = false;
                            for chunk in values.chunks(128) {
                                context.checkpoint()?;
                                if chunk.contains(needle) { found = true; break; }
                            }
                            found
                        },
                        _ => return Err(context.error(NativeError::type_error("Assert::contains expects a string substring and string, or a value and list"))),
                    },
                    _ => unreachable!(),
                };
                if passed { return Ok(Value::Null); }
                let message = if name == "same" || name == "notSame" {
                    format!("Assert::{name}() failed\nExpected: {}\nActual:   {}", describe(&values[0]), describe(&values[1]))
                } else { format!("Assert::{name}() failed: {}", values.iter().take(message_index).map(describe).collect::<Vec<_>>().join(", ")) };
                failure(context, &state, with_message(message, values, message_index))
            },
        )).map_err(|e| e.to_string())?;
    }
    let failed = state.clone();
    runtime
        .register(
            "Assert::fail",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::String], NativeType::Null),
                move |context, values| {
                    let Value::String(message) = &values[0] else {
                        unreachable!()
                    };
                    failure(context, &failed, clipped(message, 4096))
                },
            ),
        )
        .map_err(|e| e.to_string())?;
    let failed = state.clone();
    runtime.register("Assert::throws", NativeFunction::reentrant(
        NativeSignature::new(vec![NativeType::Callable, NativeType::String], NativeType::Null).with_optional(vec![NativeType::String]),
        move |context, values| {
            let Value::String(expected) = &values[1] else { unreachable!() };
            match context.invoke(values[0].clone(), &[]) {
                Ok(_) => failure(context, &failed, with_message(format!("Assert::throws() expected {expected}, but no exception was thrown"), values, 2)),
                Err(error) => match error.exception_kind() {
                    Some("ResourceLimitError") => Err(error),
                    Some(kind) if kind == expected => Ok(Value::Null),
                    Some(kind) => failure(context, &failed, with_message(format!("Assert::throws() expected {expected}, got {kind}"), values, 2)),
                    None => Err(error),
                },
            }
        },
    )).map_err(|e| e.to_string())?;
    runtime
        .register(
            "Test::skip",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::String], NativeType::Null),
                move |context, values| {
                    let Value::String(reason) = &values[0] else {
                        unreachable!()
                    };
                    state.borrow_mut().skipped = Some(clipped(reason, 4096));
                    Err(context.error(NativeError::new("Interrupted", "test skipped")))
                },
            ),
        )
        .map_err(|e| e.to_string())?;
    runtime
        .register(
            "Test::tempDir",
            NativeFunction::reentrant(
                NativeSignature::new(vec![], NativeType::String),
                move |context, _| {
                    context.require_permission("filesystem.write")?;
                    let path = temp_dir.to_str().ok_or_else(|| {
                        context.error(NativeError::new(
                            "IoError",
                            "test temporary path is not UTF-8",
                        ))
                    })?;
                    Ok(Value::String(path.into()))
                },
            ),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
