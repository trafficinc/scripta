//! UTF-8 HTML text and quoted-attribute escaping with bounded output.
use crate::bytes::{grow, resource};
use scr_runtime::{
    NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, RuntimeContext, Value,
};
use std::mem::size_of;

fn entity(character: char) -> Option<&'static str> {
    match character {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '"' => Some("&quot;"),
        '\'' => Some("&#039;"),
        _ => None,
    }
}

fn checkpoint(context: &RuntimeContext<'_>, index: usize) -> NativeResult<()> {
    if index % 1024 == 0 {
        context.check_cancelled()?;
    }
    Ok(())
}

fn escape(context: &RuntimeContext<'_>, input: &str) -> NativeResult {
    context.check_cancelled()?;
    let mut total = size_of::<Value>();
    // Validate the exact expanded size before allocating, including empty input.
    grow(context, &mut total, 0)?;
    for (index, character) in input.chars().enumerate() {
        checkpoint(context, index)?;
        grow(
            context,
            &mut total,
            entity(character).map_or(character.len_utf8(), str::len),
        )?;
    }
    context.check_cancelled()?;
    let mut output = String::new();
    output
        .try_reserve_exact(total - size_of::<Value>())
        .map_err(|_| resource("HTML escape allocation failed"))?;
    for (index, character) in input.chars().enumerate() {
        checkpoint(context, index)?;
        if let Some(entity) = entity(character) {
            output.push_str(entity);
        } else {
            output.push(character);
        }
    }
    context.check_cancelled()?;
    Ok(Value::String(output))
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Html::escape",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::String),
            |context, args| {
                let Value::String(input) = &args[0] else {
                    unreachable!("validated string parameter")
                };
                escape(context, input)
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn native_escape_checks_cancellation_without_relying_on_evaluator_checkpoints() {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        let mut context = RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: Some(Arc::new(AtomicBool::new(true))),
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        };
        for text in ["", "plain", "<&'\""] {
            assert_eq!(escape(&context, text).unwrap_err().kind, "Interrupted");
        }
        context.cancellation = None;
        context.max_value_bytes = Some(size_of::<Value>());
        assert_eq!(escape(&context, "").unwrap(), Value::String(String::new()));
        context.max_value_bytes = Some(size_of::<Value>() - 1);
        assert_eq!(escape(&context, "").unwrap_err().kind, "ResourceLimitError");
        assert!(output.is_empty());
    }
}
