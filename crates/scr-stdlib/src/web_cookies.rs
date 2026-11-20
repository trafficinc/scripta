use super::{Response, error};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{cell::RefCell, rc::Rc};
fn failure(message: &str) -> NativeError {
    NativeError::new("CookieError", message)
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
/// Parse raw Cookie fields as ordered pairs. Values retain their wire encoding;
/// this is also used for opaque session IDs, which are never URL parameters.
pub fn request_cookies(headers: &[(String, String)]) -> NativeResult<Vec<(String, String)>> {
    let mut result = vec![];
    for (_, header) in headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("cookie"))
    {
        for pair in header.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| failure("malformed Cookie header"))?;
            let name = name.trim();
            let value = value.trim();
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };
            if !valid_name(name)
                || !value.bytes().all(|b| {
                    b == 0x21
                        || (0x23..=0x2b).contains(&b)
                        || (0x2d..=0x3a).contains(&b)
                        || (0x3c..=0x5b).contains(&b)
                        || (0x5d..=0x7e).contains(&b)
                })
            {
                return Err(failure("invalid cookie name or value"));
            }
            if result.len() >= 256 {
                return Err(failure("request cookie count exceeds 256"));
            }
            result.push((name.into(), value.into()));
        }
    }
    Ok(result)
}
pub(super) fn read(
    context: &RuntimeContext<'_>,
    headers: &[(String, String)],
    key: Option<&Value>,
) -> NativeResult {
    let pairs = request_cookies(headers)?;
    if let Some(Value::String(key)) = key {
        if !valid_name(key) {
            return Err(failure("invalid cookie name"));
        }
        for (name, value) in pairs {
            context.check_cancelled()?;
            if &name == key {
                return crate::url::invoke(
                    "Url::decodeComponent",
                    context,
                    &[Value::String(value)],
                );
            }
        }
        return Ok(Value::Null);
    }
    let mut total = std::mem::size_of::<Value>();
    let mut result = Vec::new();
    for (name, value) in pairs {
        context.check_cancelled()?;
        let value = crate::url::invoke("Url::decodeComponent", context, &[Value::String(value)])?;
        let Value::String(decoded) = &value else {
            unreachable!()
        };
        crate::bytes::grow(
            context,
            &mut total,
            3 * std::mem::size_of::<Value>() + name.len() + decoded.len(),
        )?;
        result.push(Value::List(vec![Value::String(name), value]));
    }
    Ok(Value::List(result))
}
#[derive(Clone)]
struct Options {
    path: String,
    domain: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: String,
    max_age: Option<i64>,
    expires: Option<i64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            path: "/".into(),
            domain: None,
            secure: true,
            http_only: true,
            same_site: "Lax".into(),
            max_age: None,
            expires: None,
        }
    }
}
fn options(value: Option<&Value>) -> NativeResult<Options> {
    let mut options = Options::default();
    let entries = match value {
        None => return Ok(options),
        Some(Value::List(v)) if v.is_empty() => return Ok(options),
        Some(Value::Map(v)) => v,
        _ => {
            return Err(NativeError::type_error(
                "cookie options must be a map or empty list",
            ));
        }
    };
    for (key, value) in entries {
        match (key, value) {
            (MapKey::String(key), Value::String(value)) if key == "path" => {
                options.path = value.clone()
            }
            (MapKey::String(key), Value::String(value)) if key == "domain" => {
                options.domain = Some(value.trim_start_matches('.').to_ascii_lowercase())
            }
            (MapKey::String(key), Value::String(value)) if key == "sameSite" => {
                options.same_site = match value.to_ascii_lowercase().as_str() {
                    "lax" => "Lax",
                    "strict" => "Strict",
                    "none" => "None",
                    _ => return Err(failure("sameSite must be Lax, Strict, or None")),
                }
                .into()
            }
            (MapKey::String(key), Value::Bool(value)) if key == "secure" => options.secure = *value,
            (MapKey::String(key), Value::Bool(value)) if key == "httpOnly" => {
                options.http_only = *value
            }
            (MapKey::String(key), Value::Int(value))
                if key == "maxAge" && (0..=34_560_000).contains(value) =>
            {
                options.max_age = Some(*value)
            }
            (MapKey::String(key), Value::Int(value))
                if key == "expires" && (0..=253_402_300_799).contains(value) =>
            {
                options.expires = Some(*value)
            }
            _ => {
                return Err(failure(
                    "unknown cookie option, invalid type, or out-of-range value",
                ));
            }
        }
    }
    if !options.path.starts_with('/')
        || options.path.len() > 1024
        || !options
            .path
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) && b != b';' && b != b'\\')
    {
        return Err(failure(
            "cookie path must be an ASCII absolute path without controls or semicolons",
        ));
    }
    if let Some(domain) = &options.domain {
        if domain.len() > 253
            || domain.is_empty()
            || !domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(failure("invalid cookie domain"));
        }
    }
    if options.same_site == "None" && !options.secure {
        return Err(failure("SameSite=None requires Secure"));
    }
    Ok(options)
}
fn render(name: &str, value: &str, options: &Options, delete: bool) -> NativeResult<String> {
    if !valid_name(name) {
        return Err(failure("invalid cookie name"));
    }
    if (name.starts_with("__Secure-") || name.starts_with("__Host-") || name.starts_with("__Http-"))
        && !options.secure
    {
        return Err(failure("cookie prefix requires Secure"));
    }
    if name.starts_with("__Host-") && (options.domain.is_some() || options.path != "/") {
        return Err(failure("__Host- cookies require Path=/ and no Domain"));
    }
    if (name.starts_with("__Http-") || name.starts_with("__Host-Http-")) && !options.http_only {
        return Err(failure("cookie prefix requires HttpOnly"));
    }
    let value = if delete { "" } else { value };
    let mut result = format!("{name}={value}; Path={}", options.path);
    if let Some(domain) = &options.domain {
        result.push_str("; Domain=");
        result.push_str(domain);
    }
    if delete {
        result.push_str("; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
    } else {
        if let Some(age) = options.max_age {
            result.push_str(&format!("; Max-Age={age}"));
        }
        if let Some(expires) = options.expires {
            let date = chrono::DateTime::<chrono::Utc>::from_timestamp(expires, 0)
                .ok_or_else(|| failure("invalid cookie expiry"))?;
            result.push_str(&format!(
                "; Expires={}",
                date.format("%a, %d %b %Y %H:%M:%S GMT")
            ));
        }
    }
    if options.secure {
        result.push_str("; Secure");
    }
    if options.http_only {
        result.push_str("; HttpOnly");
    }
    result.push_str("; SameSite=");
    result.push_str(&options.same_site);
    if result.len() > 4096 {
        return Err(failure("Set-Cookie exceeds 4096 bytes"));
    }
    Ok(result)
}
pub(super) fn managed_session_cookie(
    response: &mut Response,
    id: &str,
    secure: bool,
    ttl: u64,
    delete: bool,
) -> NativeResult<()> {
    let value = render(
        "SCRIPTA_SESSION",
        id,
        &Options {
            secure,
            max_age: Some(ttl as i64),
            ..Default::default()
        },
        delete,
    )?;
    let mut candidate = response.clone();
    candidate
        .headers
        .retain(|(key, value)| key != "set-cookie" || !value.starts_with("SCRIPTA_SESSION="));
    candidate.header("Set-Cookie", &value, true)?;
    candidate.header("Cache-Control", "no-store", false)?;
    *response = candidate;
    Ok(())
}
pub(super) fn register(runtime: &mut Runtime, response: Rc<RefCell<Response>>) -> NativeResult<()> {
    for (name, delete) in [("Web::cookie", false), ("Web::deleteCookie", true)] {
        let response = response.clone();
        let mut parameters = vec![NativeType::String];
        if !delete {
            parameters.push(NativeType::String);
        }
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(parameters, NativeType::Null)
                    .with_optional(vec![NativeType::Any]),
                move |context, args| {
                    context.check_cancelled()?;
                    let Value::String(name) = &args[0] else {
                        unreachable!()
                    };
                    let options = options(args.get(if delete { 1 } else { 2 }))?;
                    let value = if delete {
                        String::new()
                    } else {
                        // The worst-case encoded size must fit in one cookie before allocating it.
                        let Value::String(value) = &args[1] else {
                            unreachable!()
                        };
                        if value.len() > 4096 {
                            return Err(failure("cookie value exceeds 4096 bytes"));
                        }
                        let Value::String(encoded) = crate::url::invoke(
                            "Url::encodeComponent",
                            context,
                            &[Value::String(value.clone())],
                        )?
                        else {
                            unreachable!()
                        };
                        encoded
                    };
                    let cookie = render(name, &value, &options, delete)?;
                    response
                        .borrow_mut()
                        .header("Set-Cookie", &cookie, true)
                        .map_err(|_| error("response header limit or validation failed"))?;
                    Ok(Value::Null)
                },
            ),
        )?;
    }
    Ok(())
}
