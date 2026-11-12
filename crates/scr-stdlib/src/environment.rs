use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::ffi::{OsStr, OsString};

pub(crate) fn name(name: &str) -> NativeResult<()> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(NativeError::type_error(
            "environment name must be nonempty and contain neither '=' nor NUL",
        ));
    }
    Ok(())
}
pub(crate) fn value(value: &str) -> NativeResult<()> {
    if value.contains('\0') {
        return Err(NativeError::type_error("environment value contains NUL"));
    }
    Ok(())
}
pub(crate) fn key(
    environment: &std::collections::BTreeMap<OsString, OsString>,
    name: &str,
) -> OsString {
    #[cfg(windows)]
    if let Some(key) = environment.keys().find(|key| {
        key.to_str()
            .is_some_and(|key| key.eq_ignore_ascii_case(name))
    }) {
        return key.clone();
    }
    #[cfg(not(windows))]
    let _ = environment;
    OsString::from(name)
}
pub(crate) fn text(value: &OsStr) -> NativeResult<String> {
    value.to_str().map(str::to_owned).ok_or_else(|| {
        NativeError::new(
            "EncodingError",
            "environment or CLI value is not valid UTF-8",
        )
    })
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    for (method, params, returns) in [
        (
            "get",
            vec![NativeType::String],
            NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
        ),
        ("has", vec![NativeType::String], NativeType::Bool),
        (
            "set",
            vec![NativeType::String, NativeType::String],
            NativeType::Null,
        ),
        ("remove", vec![NativeType::String], NativeType::Null),
        ("all", vec![], NativeType::Map),
    ] {
        runtime.register(
            format!("Env::{method}"),
            NativeFunction::new(
                NativeSignature::new(params, returns),
                move |context, args| {
                    context
                        .permissions
                        .require(if matches!(method, "set" | "remove") {
                            "environment.write"
                        } else {
                            "environment.read"
                        })?;
                    if method == "all" {
                        let mut entries = context
                            .environment
                            .iter()
                            .map(|(key, value)| {
                                Ok((MapKey::String(text(key)?), Value::String(text(value)?)))
                            })
                            .collect::<NativeResult<Vec<_>>>()?;
                        entries.sort_by(|left, right| match (&left.0, &right.0) {
                            (MapKey::String(a), MapKey::String(b)) => a.cmp(b),
                            _ => std::cmp::Ordering::Equal,
                        });
                        return Ok(Value::Map(entries));
                    }
                    let Some(Value::String(variable)) = args.first() else {
                        return Err(NativeError::type_error("environment name must be a string"));
                    };
                    name(variable)?;
                    let key = key(context.environment, variable);
                    match method {
                        "get" => context
                            .environment
                            .get(&key)
                            .map_or(Ok(Value::Null), |value| text(value).map(Value::String)),
                        "has" => Ok(Value::Bool(context.environment.contains_key(&key))),
                        "remove" => {
                            context.environment.remove(&key);
                            Ok(Value::Null)
                        }
                        "set" => {
                            let Some(Value::String(contents)) = args.get(1) else {
                                return Err(NativeError::type_error(
                                    "environment value must be a string",
                                ));
                            };
                            value(contents)?;
                            context.environment.insert(key, contents.into());
                            Ok(Value::Null)
                        }
                        _ => Err(NativeError::new(
                            "NativeError",
                            "unknown environment operation",
                        )),
                    }
                },
            ),
        )?;
    }
    Ok(())
}
