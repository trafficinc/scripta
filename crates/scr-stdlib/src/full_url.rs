//! Bounded immutable WHATWG URL values, independent of HTTP and path policies.
use crate::bytes::{payload, resource};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{fmt, mem::size_of, rc::Rc};
use url::{Origin, Url};

const MAX_URL_BYTES: usize = 64 * 1024;

struct Parsed {
    url: Url,
    origin: Option<String>,
}
impl fmt::Debug for Parsed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug must not reveal passwords, usernames, query tokens, or URL paths.
        f.write_str("Url { .. }")
    }
}
fn text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |value| Value::String(value.into()))
}
fn port(value: Option<u16>) -> Value {
    value.map_or(Value::Null, |value| Value::Int(i64::from(value)))
}
impl NativeObject for Parsed {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "Url"
    }
    fn payload_bytes(&self) -> usize {
        size_of::<Self>() + self.url.as_str().len() + self.origin.as_ref().map_or(0, String::len)
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        Ok(match name {
            "scheme" => Value::String(self.url.scheme().into()),
            "username" => Value::String(self.url.username().into()),
            "password" => text(self.url.password()),
            "host" => text(self.url.host_str()),
            "port" => port(self.url.port()),
            "effectivePort" => port(self.url.port_or_known_default()),
            "path" => Value::String(self.url.path().into()),
            "query" => text(self.url.query()),
            "fragment" => text(self.url.fragment()),
            "origin" => text(self.origin.as_deref()),
            "canBeBase" => Value::Bool(!self.url.cannot_be_a_base()),
            _ => return Err(format!("unknown Url property {name}")),
        })
    }
}

fn invalid(message: &str) -> NativeError {
    NativeError::new("UrlError", message)
}
fn validate(context: &RuntimeContext<'_>, input: &str) -> NativeResult<()> {
    context.check_cancelled()?;
    if input.len() > MAX_URL_BYTES {
        return Err(resource("URL input exceeds 65536 UTF-8 bytes"));
    }
    let bytes = input.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if index % 1024 == 0 {
            context.check_cancelled()?;
        }
        if byte <= b' ' || byte == 127 || byte == b'\\' {
            return Err(invalid("URL contains a raw control, space, or backslash"));
        }
        if byte == b'%'
            && !bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(invalid("URL contains an invalid percent escape"));
        }
    }
    Ok(())
}
fn parse(context: &RuntimeContext<'_>, input: &str, base: Option<&Url>) -> NativeResult<Url> {
    validate(context, input)?;
    let result = match base {
        Some(base) => base.join(input),
        None => Url::parse(input),
    };
    context.check_cancelled()?;
    // ParseError contains a static category, never the supplied URL/credentials.
    let url = result.map_err(|error| {
        NativeError::new("UrlError", format!("invalid URL or reference: {error}"))
    })?;
    if url.as_str().len() > MAX_URL_BYTES {
        return Err(resource("canonical URL exceeds 65536 UTF-8 bytes"));
    }
    Ok(url)
}
fn publish(context: &RuntimeContext<'_>, url: Url) -> NativeResult<Value> {
    // The parser's allocation is bounded by its input cap. Charge its retained
    // text before computing/caching origin, then charge the complete result.
    payload(context, size_of::<Parsed>() + url.as_str().len())?;
    context.check_cancelled()?;
    let origin = match url.origin() {
        origin @ Origin::Tuple(..) => Some(origin.ascii_serialization()),
        Origin::Opaque(_) => None,
    };
    context.check_cancelled()?;
    let parsed = Parsed { url, origin };
    payload(context, parsed.payload_bytes())?;
    Ok(Value::NativeObject(Rc::new(parsed)))
}
fn object(value: &Value) -> NativeResult<&Parsed> {
    let Value::NativeObject(value) = value else {
        return Err(NativeError::type_error("expected a Url"));
    };
    value
        .as_any()
        .and_then(|value| value.downcast_ref::<Parsed>())
        .ok_or_else(|| NativeError::type_error("expected a real parsed Url"))
}
fn string(value: &Value) -> &str {
    let Value::String(value) = value else {
        unreachable!("validated string")
    };
    value
}
fn invoke(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult<Value> {
    context.check_cancelled()?;
    match name {
        "parse" | "resolve" => {
            payload(context, size_of::<Parsed>())?;
            let url = if name == "parse" {
                parse(context, string(&args[0]), None)?
            } else {
                parse(context, string(&args[1]), Some(&object(&args[0])?.url))?
            };
            publish(context, url)
        }
        "serialize" => {
            let url = &object(&args[0])?.url;
            payload(context, url.as_str().len())?;
            Ok(Value::String(url.as_str().into()))
        }
        "isValid" => {
            payload(context, 0)?;
            match parse(context, string(&args[0]), None) {
                Ok(_) => Ok(Value::Bool(true)),
                Err(error) if error.kind == "UrlError" => Ok(Value::Bool(false)),
                Err(error) => Err(error),
            }
        }
        _ => unreachable!(),
    }
}
pub(super) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bool, Object, String as Text};
    for (name, parameters, returns) in [
        ("parse", vec![Text], Object("Url")),
        ("serialize", vec![Object("Url")], Text),
        ("resolve", vec![Object("Url"), Text], Object("Url")),
        ("isValid", vec![Text], Bool),
    ] {
        runtime.register(
            format!("Url::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| invoke(name, context, args),
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }
    fn string(value: &str) -> Value {
        Value::String(value.into())
    }
    #[test]
    fn exact_url_payloads_include_serialization_and_cached_origins() {
        with_context(|context| {
            for (input, canonical, origin) in [
                (
                    "https://EXAMPLE.test:443/a/../b?x=1#f",
                    "https://example.test/b?x=1#f",
                    "https://example.test",
                ),
                ("mailto:user@example.test", "mailto:user@example.test", ""),
                ("file:///tmp/a", "file:///tmp/a", ""),
            ] {
                let required =
                    size_of::<Value>() + size_of::<Parsed>() + canonical.len() + origin.len();
                context.max_value_bytes = Some(required);
                let value = invoke("parse", context, &[string(input)]).unwrap();
                assert_eq!(
                    object(&value).unwrap().payload_bytes() + size_of::<Value>(),
                    required
                );
                context.max_value_bytes = Some(required - 1);
                assert_eq!(
                    invoke("parse", context, &[string(input)]).unwrap_err().kind,
                    "ResourceLimitError"
                );
                context.max_value_bytes = Some(size_of::<Value>() + canonical.len());
                assert_eq!(
                    invoke("serialize", context, std::slice::from_ref(&value)).unwrap(),
                    string(canonical)
                );
                context.max_value_bytes = Some(size_of::<Value>() + canonical.len() - 1);
                assert_eq!(
                    invoke("serialize", context, std::slice::from_ref(&value))
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
                // Validity needs a bool result slot, not a retained Url payload.
                context.max_value_bytes = Some(size_of::<Value>());
                assert_eq!(
                    invoke("isValid", context, &[string(input)]).unwrap(),
                    Value::Bool(true)
                );
                context.max_value_bytes = Some(size_of::<Value>() - 1);
                assert_eq!(
                    invoke("isValid", context, &[string(input)])
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
            }
            context.max_value_bytes = None;
            let base = invoke("parse", context, &[string("https://example.test/a/b")]).unwrap();
            let canonical = "https://example.test/summary";
            let required = size_of::<Value>()
                + size_of::<Parsed>()
                + canonical.len()
                + "https://example.test".len();
            context.max_value_bytes = Some(required);
            let resolved =
                invoke("resolve", context, &[base.clone(), string("../summary")]).unwrap();
            assert_eq!(object(&resolved).unwrap().url.as_str(), canonical);
            context.max_value_bytes = Some(required - 1);
            assert_eq!(
                invoke("resolve", context, &[base.clone(), string("../summary")])
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            assert_eq!(
                object(&base).unwrap().url.as_str(),
                "https://example.test/a/b"
            );
        });
    }
    #[test]
    fn full_url_operations_propagate_cancellation_including_invalid_predicates() {
        with_context(|context| {
            let base = invoke("parse", context, &[string("https://example.test/a")]).unwrap();
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            for (name, args) in [
                ("parse", vec![string("https://example.test/")]),
                ("resolve", vec![base.clone(), string("../b")]),
                ("serialize", vec![base]),
                ("isValid", vec![string("invalid")]),
            ] {
                assert_eq!(
                    invoke(name, context, &args).unwrap_err().kind,
                    "Interrupted",
                    "{name}"
                );
            }
        });
    }
}
