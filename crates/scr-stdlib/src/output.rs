//! Host/transport flushing through the native interface.
use scr_runtime::{NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value};
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
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
