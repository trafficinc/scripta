//! Explicit application buffering and transport flushing through the native interface.
use scr_runtime::{
    NativeFunction, NativeResult, NativeSignature, NativeType, OutputAction, Runtime, Value,
};
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use OutputAction::*;
    for (name, action, returns) in [
        ("ob_start", Start, NativeType::Null),
        ("ob_flush", Flush, NativeType::Null),
        ("ob_end_flush", EndFlush, NativeType::Null),
        ("ob_clean", Clean, NativeType::Null),
        ("ob_end_clean", EndClean, NativeType::Null),
        ("ob_get_contents", Contents, NativeType::Any),
        ("ob_get_clean", GetClean, NativeType::Bytes),
        ("ob_get_flush", GetFlush, NativeType::Bytes),
        ("ob_get_length", Length, NativeType::Any),
        ("ob_get_level", Level, NativeType::Int),
    ] {
        runtime.register(
            name,
            NativeFunction::new(NativeSignature::new(vec![], returns), move |context, _| {
                context.check_cancelled()?;
                context.output.control(action)
            }),
        )?;
    }
    runtime.register(
        "flush",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Null),
            |context, _| {
                context.check_cancelled()?;
                crate::bytes::budget(context, size_of::<Value>())?;
                context.output.flush()?;
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(())
}
