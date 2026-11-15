//! Request-scoped web primitives. Hosts explicitly register these for each request.
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    Value,
};
use std::{cell::RefCell, rc::Rc};
#[path = "web_cookies.rs"]
mod cookies;
#[path = "web_sessions.rs"]
pub mod session;
pub use cookies::request_cookies;
use ureq::http::{HeaderName, HeaderValue, Uri};

/// A bounded, fully received HTTP request supplied by a trusted host.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    /// Encoded path, excluding the query. No implicit percent decoding.
    pub path: String,
    pub query_string: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
/// Response metadata; body output goes to the runtime's bounded output writer.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    max_header_bytes: usize,
}
fn error(message: &str) -> NativeError {
    NativeError::new("WebError", message)
}
fn text(bytes: &[u8]) -> NativeResult<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| NativeError::new("EncodingError", "request body is not UTF-8"))
}
#[derive(Debug)]
struct RequestObject(Rc<Request>);
impl std::ops::Deref for RequestObject {
    type Target = Request;
    fn deref(&self) -> &Request {
        &self.0
    }
}
impl NativeObject for RequestObject {
    fn type_name(&self) -> &str {
        "WebRequest"
    }
    fn payload_bytes(&self) -> usize {
        self.method
            .len()
            .saturating_add(self.path.len())
            .saturating_add(self.query_string.len())
            .saturating_add(self.body.len())
            .saturating_add(
                self.headers
                    .iter()
                    .map(|(k, v)| k.len() + v.len() + 2 * std::mem::size_of::<String>())
                    .sum::<usize>(),
            )
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "method" => return Ok(Value::String(self.method.clone())),
            "path" => return Ok(Value::String(self.path.clone())),
            "queryString" => return Ok(Value::String(self.query_string.clone())),
            _ => {}
        }
        let name = name.to_owned();
        let (parameters, returns) = match name.as_str() {
            "header" | "cookie" => (vec![NativeType::String], NativeType::Any),
            "cookies" => (vec![], NativeType::List),
            "headers" => (vec![NativeType::String], NativeType::List),
            "query" | "form" => (vec![], NativeType::List),
            "json" => (vec![], NativeType::Any),
            "text" => (vec![], NativeType::String),
            "bytes" => (vec![], NativeType::Bytes),
            _ => return Err(format!("unknown WebRequest property {name}")),
        };
        let request = self.0.clone();
        Ok(Value::native_function(
            format!("WebRequest::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| {
                    context.check_cancelled()?;
                    let headers = &request.headers;
                    let body = &request.body;
                    let query = &request.query_string;
                    match name.as_str() {
                        "cookie" | "cookies" => cookies::read(context, headers, args.first()),
                        "header" | "headers" => {
                            let Value::String(key) = &args[0] else {
                                unreachable!()
                            };
                            HeaderName::from_bytes(key.as_bytes())
                                .map_err(|_| error("invalid request header name"))?;
                            let mut values = headers
                                .iter()
                                .filter(|(k, _)| k.eq_ignore_ascii_case(key))
                                .map(|(_, v)| Value::String(v.clone()));
                            Ok(if name == "header" {
                                values.next().unwrap_or(Value::Null)
                            } else {
                                Value::List(values.collect())
                            })
                        }
                        "query" => crate::url::parse(context, query, true),
                        "bytes" => {
                            crate::bytes::payload(context, body.len())?;
                            Ok(Value::Bytes(body.clone().into()))
                        }
                        "text" => {
                            crate::bytes::payload(context, body.len())?;
                            Ok(Value::String(text(body)?.into()))
                        }
                        "json" | "form" => {
                            let types: Vec<_> = headers
                                .iter()
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                                .collect();
                            if types.len() != 1 {
                                return Err(error("exactly one Content-Type is required"));
                            }
                            let media = types[0]
                                .1
                                .split(';')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_ascii_lowercase();
                            if name == "form" {
                                if media != "application/x-www-form-urlencoded" {
                                    return Err(error(
                                        "form() requires application/x-www-form-urlencoded",
                                    ));
                                }
                                crate::url::parse(context, text(body)?, true)
                            } else {
                                if media != "application/json"
                                    && !(media.starts_with("application/")
                                        && media.ends_with("+json"))
                                {
                                    return Err(error(
                                        "json() requires an application JSON Content-Type",
                                    ));
                                }
                                crate::json::decode(text(body)?)
                            }
                        }
                        _ => unreachable!(),
                    }
                },
            ),
        ))
    }
}
impl Response {
    fn header(&mut self, name: &str, value: &str, append: bool) -> NativeResult<()> {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| error("invalid response header name"))?
            .as_str()
            .to_owned();
        HeaderValue::from_str(value).map_err(|_| error("invalid response header value"))?;
        if matches!(
            name.as_str(),
            "content-length"
                | "transfer-encoding"
                | "connection"
                | "keep-alive"
                | "upgrade"
                | "trailer"
                | "te"
                | "proxy-connection"
                | "proxy-authenticate"
                | "proxy-authorization"
        ) {
            return Err(error("response transport headers are managed by the host"));
        }
        let retained: usize = self
            .headers
            .iter()
            .filter(|(k, _)| append || k != &name)
            .map(|(k, v)| k.len() + v.len() + 4)
            .sum();
        if retained
            .saturating_add(name.len())
            .saturating_add(value.len())
            .saturating_add(4)
            > self.max_header_bytes
            || self.headers.len() >= 100
                && (append || !self.headers.iter().any(|(k, _)| k == &name))
        {
            return Err(error("response header limit exceeded"));
        }
        if !append {
            self.headers.retain(|(key, _)| key != &name);
        }
        self.headers.push((name, value.into()));
        Ok(())
    }
}
/// Install the Web module in a fresh runtime, returning request-owned response state.
/// These APIs are deliberately absent in ordinary CLI execution.
pub fn register(
    runtime: &mut Runtime,
    request: Request,
    max_header_bytes: usize,
) -> NativeResult<Rc<RefCell<Response>>> {
    let response = Rc::new(RefCell::new(Response {
        status: 200,
        headers: vec![],
        max_header_bytes,
    }));
    cookies::register(runtime, response.clone())?;
    let request: Rc<dyn NativeObject> = Rc::new(RequestObject(Rc::new(request)));
    runtime.register(
        "Web::request",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Object("WebRequest")),
            move |context, _| {
                context.check_cancelled()?;
                Ok(Value::NativeObject(request.clone()))
            },
        ),
    )?;
    let state = response.clone();
    runtime.register(
        "Web::status",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Null),
            move |context, args| {
                context.check_cancelled()?;
                let Value::Int(code) = args[0] else {
                    unreachable!()
                };
                if !(200..=599).contains(&code) {
                    return Err(error("response status must be between 200 and 599"));
                }
                state.borrow_mut().status = code as u16;
                Ok(Value::Null)
            },
        ),
    )?;
    for (name, append) in [("Web::header", false), ("Web::addHeader", true)] {
        let state = response.clone();
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::String, NativeType::String],
                    NativeType::Null,
                ),
                move |context, args| {
                    context.check_cancelled()?;
                    let [Value::String(key), Value::String(value)] = args else {
                        unreachable!()
                    };
                    state.borrow_mut().header(key, value, append)?;
                    Ok(Value::Null)
                },
            ),
        )?;
    }
    let state = response.clone();
    runtime.register(
        "Web::redirect",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Null)
                .with_optional(vec![NativeType::Int]),
            move |context, args| {
                context.check_cancelled()?;
                let Value::String(location) = &args[0] else {
                    unreachable!()
                };
                let code = if let Some(Value::Int(code)) = args.get(1) {
                    *code
                } else {
                    302
                };
                if ![301, 302, 303, 307, 308].contains(&code) {
                    return Err(error("redirect status must be 301, 302, 303, 307, or 308"));
                }
                let uri: Uri = location
                    .parse()
                    .map_err(|_| error("invalid redirect location"))?;
                if (uri.scheme_str().is_none()
                    && location
                        .split(['/', '?', '#'])
                        .next()
                        .is_some_and(|part| part.contains(':')))
                    || location.is_empty()
                    || location.starts_with("//")
                    || location.contains('\\')
                    || uri
                        .scheme_str()
                        .is_some_and(|s| s != "http" && s != "https")
                    || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                {
                    return Err(error(
                        "redirect requires a relative URI or an HTTP(S) URL without credentials",
                    ));
                }
                let mut response = state.borrow_mut();
                response.header("Location", location, false)?;
                response.status = code as u16;
                Ok(Value::Null)
            },
        ),
    )?;
    runtime.register(
        "Web::write",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::OneOf(vec![
                    NativeType::String,
                    NativeType::Bytes,
                ])],
                NativeType::Null,
            ),
            |context, args| {
                context.check_cancelled()?;
                let bytes = match &args[0] {
                    Value::String(s) => s.as_bytes(),
                    Value::Bytes(b) => b,
                    _ => unreachable!(),
                };
                context.output.write_all(bytes)?;
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(response)
}
