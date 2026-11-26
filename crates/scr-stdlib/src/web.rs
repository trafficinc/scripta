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
#[path = "web_uploads.rs"]
pub mod uploads;
pub use cookies::request_cookies;
use ureq::http::{HeaderName, HeaderValue, Uri};

/// Connection metadata resolved by a trusted HTTP host, never inferred by the
/// standard library from raw request headers. Embeddings may leave fields unknown.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ConnectionInfo {
    pub peer_address: Option<std::net::SocketAddr>,
    pub client_address: Option<std::net::IpAddr>,
    pub scheme: Option<String>,
    pub host: Option<String>,
}
impl ConnectionInfo {
    fn payload_bytes(&self) -> usize {
        self.scheme
            .as_ref()
            .map_or(0, String::len)
            .saturating_add(self.host.as_ref().map_or(0, String::len))
    }
}

/// A bounded, fully received HTTP request supplied by a trusted host.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    /// Encoded path, excluding the query. No implicit percent decoding.
    pub path: String,
    pub query_string: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub connection: ConnectionInfo,
    pub request_id: Option<String>,
}
/// Response metadata; body output goes to the runtime's bounded output writer.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    max_header_bytes: usize,
    committed: bool,
}
fn error(message: &str) -> NativeError {
    NativeError::new("WebError", message)
}
fn text(bytes: &[u8]) -> NativeResult<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| NativeError::new("EncodingError", "request body is not UTF-8"))
}
#[derive(Debug)]
struct RequestObject {
    owner: Rc<()>,
    request: Rc<Request>,
    multipart: Option<Rc<uploads::State>>,
    inputs: Rc<crate::streams::InputScope>,
}
impl Drop for RequestObject {
    fn drop(&mut self) {
        self.inputs.close();
    }
}
impl std::ops::Deref for RequestObject {
    type Target = Request;
    fn deref(&self) -> &Request {
        &self.request
    }
}
impl NativeObject for RequestObject {
    fn type_name(&self) -> &str {
        "WebRequest"
    }
    fn payload_bytes(&self) -> usize {
        self.multipart
            .as_ref()
            .map_or(0, |m| m.payload_bytes())
            .saturating_add(
                self.method
                    .len()
                    .saturating_add(self.path.len())
                    .saturating_add(self.query_string.len())
                    .saturating_add(self.body.len())
                    .saturating_add(self.connection.payload_bytes())
                    .saturating_add(self.request_id.as_ref().map_or(0, String::len))
                    .saturating_add(
                        self.headers
                            .iter()
                            .map(|(k, v)| k.len() + v.len() + 2 * std::mem::size_of::<String>())
                            .sum::<usize>(),
                    ),
            )
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "requestId" => return Ok(self.request_id.clone().map_or(Value::Null, Value::String)),
            "peerAddress" => {
                return Ok(self
                    .connection
                    .peer_address
                    .map_or(Value::Null, |v| Value::String(v.to_string())));
            }
            "clientAddress" => {
                return Ok(self
                    .connection
                    .client_address
                    .map_or(Value::Null, |v| Value::String(v.to_string())));
            }
            "scheme" => {
                return Ok(self
                    .connection
                    .scheme
                    .clone()
                    .map_or(Value::Null, Value::String));
            }
            "host" => {
                return Ok(self
                    .connection
                    .host
                    .clone()
                    .map_or(Value::Null, Value::String));
            }
            "method" => return Ok(Value::String(self.method.clone())),
            "path" => return Ok(Value::String(self.path.clone())),
            "queryString" => return Ok(Value::String(self.query_string.clone())),
            "bodySize" => {
                return Ok(Value::Int(
                    self.multipart
                        .as_ref()
                        .map_or(self.body.len() as u64, |m| m.body_size())
                        as i64,
                ));
            }
            _ => {}
        }
        let name = name.to_owned();
        let (parameters, returns) = match name.as_str() {
            "header" | "cookie" => (vec![NativeType::String], NativeType::Any),
            "cookies" => (vec![], NativeType::List),
            "headers" => (vec![NativeType::String], NativeType::List),
            "query" | "form" | "uploads" => (vec![], NativeType::List),
            "json" => (vec![], NativeType::Any),
            "text" => (vec![], NativeType::String),
            "bytes" => (vec![], NativeType::Bytes),
            "bodyStream" => (vec![], NativeType::Object("ByteStream")),
            _ => return Err(format!("unknown WebRequest property {name}")),
        };
        let owner = self.owner.clone();
        let request = self.request.clone();
        let multipart = self.multipart.clone();
        let inputs = self.inputs.clone();
        Ok(Value::native_function(
            format!("WebRequest::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| {
                    context.check_cancelled()?;
                    if !context
                        .runtime_identity
                        .as_ref()
                        .is_some_and(|id| Rc::ptr_eq(id, &owner))
                    {
                        return Err(error("request belongs to another runtime"));
                    }
                    if let Some(multipart) = &multipart {
                        multipart.check(context)?;
                    }
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
                        "uploads" => multipart
                            .as_ref()
                            .map_or_else(|| Ok(Value::list(vec![])), |m| m.files(context)),
                        "bodyStream" => {
                            if let Some(m) = &multipart {
                                m.body_stream(context)
                            } else {
                                let reader = MemoryBody {
                                    request: request.clone(),
                                    offset: 0,
                                };
                                crate::streams::request_input(
                                    context,
                                    Box::new(reader),
                                    request.body.len() as u64,
                                    inputs.clone(),
                                    request.body.len(),
                                )
                            }
                        }
                        "bytes" if multipart.is_some() => Ok(Value::Bytes(
                            multipart.as_ref().unwrap().body(context)?.into(),
                        )),
                        "text" if multipart.is_some() => {
                            let body = multipart.as_ref().unwrap().body(context)?;
                            Ok(Value::String(String::from_utf8(body).map_err(|_| {
                                NativeError::new("EncodingError", "request body is not UTF-8")
                            })?))
                        }
                        "form" if multipart.as_ref().is_some_and(|m| m.is_multipart()) => {
                            multipart.as_ref().unwrap().fields(context)
                        }
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
                            if name == "form" && media != "application/x-www-form-urlencoded" {
                                return Err(error(
                                    "form() requires application/x-www-form-urlencoded",
                                ));
                            }
                            if name == "json"
                                && media != "application/json"
                                && !(media.starts_with("application/") && media.ends_with("+json"))
                            {
                                return Err(error(
                                    "json() requires an application JSON Content-Type",
                                ));
                            }
                            let storage;
                            let body = if let Some(m) = &multipart {
                                storage = m.body(context)?;
                                storage.as_slice()
                            } else {
                                body
                            };
                            if name == "form" {
                                crate::url::parse(context, text(body)?, true)
                            } else {
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
struct MemoryBody {
    request: Rc<Request>,
    offset: usize,
}
impl std::io::Read for MemoryBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = buffer.len().min(self.request.body.len() - self.offset);
        buffer[..n].copy_from_slice(&self.request.body[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}
impl Response {
    /// Freeze metadata immediately before the host's first transmission.
    pub fn commit(&mut self) -> NativeResult<()> {
        self.ensure_mutable()?;
        self.committed = true;
        Ok(())
    }
    pub fn headers_committed(&self) -> bool {
        self.committed
    }
    fn ensure_mutable(&self) -> NativeResult<()> {
        if self.committed {
            Err(error("response headers have already been committed"))
        } else {
            Ok(())
        }
    }
    fn header(&mut self, name: &str, value: &str, append: bool) -> NativeResult<()> {
        self.ensure_mutable()?;
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
    register_inner(runtime, request, max_header_bytes, None)
}

/// Register a trusted host-provided multipart request and its request lifetime guard.
pub fn register_multipart(
    runtime: &mut Runtime,
    request: Request,
    data: uploads::Multipart,
    policy: crate::FilesystemPolicy,
    stage: uploads::Stage,
    max_header_bytes: usize,
) -> NativeResult<(Rc<RefCell<Response>>, uploads::RequestResources)> {
    let (state, guard) = uploads::State::new(runtime, data, policy, stage)?;
    let response = register_inner(runtime, request, max_header_bytes, Some(state))?;
    Ok((response, guard))
}
fn register_inner(
    runtime: &mut Runtime,
    request: Request,
    max_header_bytes: usize,
    multipart: Option<Rc<uploads::State>>,
) -> NativeResult<Rc<RefCell<Response>>> {
    let response = Rc::new(RefCell::new(Response {
        status: 200,
        headers: vec![],
        max_header_bytes,
        committed: false,
    }));
    let download_state = response.clone();
    let head = request.method == "HEAD";
    runtime.register("Web::download",NativeFunction::new(NativeSignature::new(vec![NativeType::Object("ByteStream"),NativeType::String],NativeType::Int),move |context,args| {
        context.check_cancelled()?;crate::bytes::budget(context,size_of::<Value>())?;
        let Value::String(filename)=&args[1] else {unreachable!()};
        if filename.is_empty() || filename.len()>255 || filename.chars().any(|c|c.is_control() || matches!(c,'/'|'\\')) {return Err(error("download filename must be a basename without controls, at most 255 UTF-8 bytes"))}
        if context.output.generated_bytes()!=Some(0) {return Err(error("download must precede body output"))}
        let size=crate::streams::download_size(context,&args[0])?;
        if !head && context.output.remaining_bytes().is_some_and(|n|size>n as u64) {return Err(crate::bytes::resource("download exceeds remaining response output budget"))}
        let encoded=crate::url::invoke("Url::encodeComponent",context,&[Value::String(filename.clone())])?;
        let Value::String(encoded)=encoded else {unreachable!()};
        let fallback=if filename.is_ascii() {filename.replace('"',"_")}else{"download".into()};
        let mut candidate=download_state.borrow().clone();
        candidate.header("Content-Type","application/octet-stream",false)?;
        candidate.header("Content-Disposition",&format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"),false)?;
        candidate.header("X-Content-Type-Options","nosniff",false)?;
        candidate.header("Accept-Ranges","none",false)?;
        candidate.status=200;
        *download_state.borrow_mut()=candidate;
        if head {context.output.flush()?;Ok(Value::Int(0))}else{crate::streams::download(context,&args[0],size)}
    }))?;
    cookies::register(runtime, response.clone())?;
    let request: Rc<dyn NativeObject> = Rc::new(RequestObject {
        owner: runtime.native_identity(),
        request: Rc::new(request),
        multipart,
        inputs: crate::streams::InputScope::new(),
    });
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
                let mut state = state.borrow_mut();
                state.ensure_mutable()?;
                state.status = code as u16;
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
