//! Type-preserving finite numeric helpers.
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};

fn calculate(name: &str, args: &[Value]) -> NativeResult {
    let invalid =
        || NativeError::type_error("Math helpers require finite numbers of the same type");
    match args {
        [Value::Int(value)] => Ok(Value::Int(match name {
            "abs" => value.checked_abs().ok_or_else(|| {
                NativeError::new("ArithmeticError", "integer absolute value overflow")
            })?,
            "floor" | "ceil" | "round" => *value,
            _ => unreachable!("validated math arity"),
        })),
        [Value::Float(value)] if value.is_finite() => Ok(Value::Float(match name {
            "abs" => value.abs(),
            "floor" => value.floor(),
            "ceil" => value.ceil(),
            "round" => value.round(),
            _ => unreachable!("validated math arity"),
        })),
        [Value::Int(left), Value::Int(right)] => Ok(Value::Int(
            if (name == "min" && right < left) || (name == "max" && right > left) {
                *right
            } else {
                *left
            },
        )),
        [Value::Float(left), Value::Float(right)] if left.is_finite() && right.is_finite() => {
            Ok(Value::Float(
                if (name == "min" && right < left) || (name == "max" && right > left) {
                    *right
                } else {
                    *left
                },
            ))
        }
        _ => Err(invalid()),
    }
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    let numeric = NativeType::OneOf(vec![NativeType::Int, NativeType::Float]);
    for name in ["abs", "min", "max", "floor", "ceil", "round"] {
        let arity = if matches!(name, "min" | "max") { 2 } else { 1 };
        runtime.register(
            format!("Math::{name}"),
            NativeFunction::new(
                NativeSignature::new(vec![numeric.clone(); arity], numeric.clone()),
                move |context, args| {
                    context.check_cancelled()?;
                    calculate(name, args)
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonfinite_host_inputs_are_rejected() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for name in ["abs", "floor", "ceil", "round"] {
                assert_eq!(
                    calculate(name, &[Value::Float(value)]).unwrap_err().kind,
                    "TypeError"
                );
            }
            for name in ["min", "max"] {
                for args in [
                    [Value::Float(value), Value::Float(1.0)],
                    [Value::Float(1.0), Value::Float(value)],
                ] {
                    assert_eq!(calculate(name, &args).unwrap_err().kind, "TypeError");
                }
            }
        }
    }
}
