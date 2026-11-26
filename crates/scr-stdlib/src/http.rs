//! Synchronous bounded HTTP with verified Rustls HTTPS and local response objects.
mod conveniences;
mod cookies;
mod multipart;
use crate::{
    bytes::{budget, grow, resource},
    filesystem::FilesystemPolicy,
    json, paths,
};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType,
    Runtime, RuntimeContext, Value,
};
use std::{
    io::{Read, Write},
    mem::size_of,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
use ureq::http::{
    self,
    header::{HeaderName, HeaderValue},
};

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub timeout: Duration,
    /// Host ceiling; per-request options may only lower this limit.
    pub max_response_bytes: u64,
    /// Host ceiling and default. Zero disables redirects.
    pub max_redirects: u32,
}
impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_response_bytes: 8 * 1024 * 1024,
            max_redirects: 5,
        }
    }
}
#[derive(Debug)]
struct Response {
    status: u16,
    headers: Value,
    body: Rc<[u8]>,
    binary: bool,
    duration_ms: f64,
}
pub(crate) fn header_bytes(headers: &Value) -> usize {
    let Value::Map(entries) = headers else {
        unreachable!()
    };
    size_of::<Value>()
        + entries
            .iter()
            .map(|(key, values)| {
                let MapKey::String(key) = key else {
                    unreachable!()
                };
                let Value::List(values) = values else {
                    unreachable!()
                };
                size_of::<MapKey>()
                    + key.len()
                    + size_of::<Value>()
                    + values
                        .iter()
                        .map(|value| {
                            let Value::String(value) = value else {
                                unreachable!()
                            };
                            size_of::<Value>() + value.len()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}
fn decode(bytes: &[u8]) -> NativeResult<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| NativeError::new("EncodingError", "HTTP response body is not UTF-8"))
}
impl NativeObject for Response {
    fn payload_bytes(&self) -> usize {
        self.body.len().saturating_add(header_bytes(&self.headers))
    }
    fn type_name(&self) -> &str {
        "HttpResponse"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        Ok(match name {
            "status" => Value::Int(i64::from(self.status)),
            "durationMs" => Value::Float(self.duration_ms),
            "headers" => self.headers.clone(),
            "body" if self.binary => Value::Bytes(self.body.clone()),
            "body" => Value::String(decode(&self.body).map_err(|e| e.to_string())?.into()),
            "ok" | "json" | "text" | "bytes" => {
                let name = name.to_owned();
                let body = self.body.clone();
                let status = self.status;
                Value::native_function(
                    format!("HttpResponse::{name}"),
                    NativeFunction::new(
                        NativeSignature::new(
                            vec![],
                            match name.as_str() {
                                "ok" => NativeType::Bool,
                                "text" => NativeType::String,
                                "bytes" => NativeType::Bytes,
                                _ => NativeType::Any,
                            },
                        ),
                        move |context, _| match name.as_str() {
                            "ok" => Ok(Value::Bool((200..300).contains(&status))),
                            "bytes" => Ok(Value::Bytes(body.clone())),
                            "text" => {
                                crate::bytes::payload(context, body.len())?;
                                Ok(Value::String(decode(&body)?.into()))
                            }
                            _ => json::decode(decode(&body)?),
                        },
                    ),
                )
            }
            _ => return Err(format!("unknown HttpResponse property {name}")),
        })
    }
}
pub(crate) fn failure(error: ureq::Error) -> NativeError {
    let kind = match &error {
        ureq::Error::Timeout(_) => "TimeoutError",
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => "TlsError",
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => "RedirectError",
        ureq::Error::BodyExceedsLimit(_) => "ResponseLimitError",
        ureq::Error::Io(error)
            if error
                .get_ref()
                .is_some_and(|inner| inner.is::<rustls::Error>()) =>
        {
            "TlsError"
        }
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            "ConnectionError"
        }
        _ => "HttpError",
    };
    NativeError::new(kind, format!("HTTP transport failed: {error}"))
}
struct Options {
    timeout: Duration,
    limit: u64,
    redirects: u32,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: Rc<[u8]>,
    binary: bool,
    jar: Option<Value>,
    retry: Option<conveniences::Retry>,
}
fn options(value: Option<&Value>, config: &HttpConfig) -> NativeResult<Options> {
    let mut result = Options {
        timeout: config.timeout,
        limit: config.max_response_bytes,
        redirects: config.max_redirects,
        headers: vec![],
        body: Rc::from([]),
        binary: false,
        jar: None,
        retry: None,
    };
    let entries = match value {
        None => return Ok(result),
        Some(Value::List(values)) if values.is_empty() => return Ok(result),
        Some(Value::Map(entries)) => entries,
        _ => {
            return Err(NativeError::type_error(
                "HTTP options must be a map or empty list",
            ));
        }
    };
    for (key, value) in entries {
        match (key, value) {
            (MapKey::String(key), Value::NativeObject(_)) if key == "cookieJar" => {
                result.jar = Some(value.clone());
            }
            (MapKey::String(key), _) if key == "retry" => {
                result.retry = Some(conveniences::Retry::parse(value)?);
            }
            (MapKey::String(key), Value::Int(ms)) if key == "timeoutMs" && *ms > 0 => {
                result.timeout = Duration::from_millis(*ms as u64)
            }
            (MapKey::String(key), Value::Int(bytes))
                if key == "maxBodyBytes"
                    && *bytes >= 0
                    && *bytes as u64 <= config.max_response_bytes =>
            {
                result.limit = *bytes as u64
            }
            (MapKey::String(key), Value::Int(count))
                if key == "maxRedirects"
                    && *count >= 0
                    && *count as u64 <= u64::from(config.max_redirects) =>
            {
                result.redirects = *count as u32
            }
            (MapKey::String(key), Value::String(body)) if key == "body" => {
                result.body = Rc::from(body.as_bytes())
            }
            (MapKey::String(key), Value::Bytes(body)) if key == "body" => {
                result.body = body.clone();
            }
            (MapKey::String(key), Value::String(mode))
                if key == "responseType" && matches!(mode.as_str(), "text" | "bytes") =>
            {
                result.binary = mode == "bytes";
            }
            (MapKey::String(key), Value::List(headers))
                if key == "headers" && headers.is_empty() => {}
            (MapKey::String(key), Value::Map(headers)) if key == "headers" => {
                for (key, value) in headers {
                    let (MapKey::String(key), Value::String(value)) = (key, value) else {
                        return Err(NativeError::type_error(
                            "HTTP headers require string names and values",
                        ));
                    };
                    let key = HeaderName::from_bytes(key.as_bytes())
                        .map_err(|_| NativeError::type_error("invalid HTTP header name"))?;
                    let value = HeaderValue::from_str(value)
                        .map_err(|_| NativeError::type_error("invalid HTTP header value"))?;
                    // Framing is determined from the actual body, not script headers.
                    if matches!(
                        key.as_str(),
                        "content-length" | "transfer-encoding" | "host"
                    ) {
                        return Err(NativeError::type_error(
                            "HTTP host and framing headers are managed by the client",
                        ));
                    }
                    result.headers.push((key, value));
                }
            }
            _ => {
                return Err(NativeError::type_error(
                    "unknown HTTP option or invalid value/host limit",
                ));
            }
        }
    }
    if Instant::now().checked_add(result.timeout).is_none() {
        return Err(NativeError::type_error(
            "HTTP timeout exceeds the platform clock range",
        ));
    }
    Ok(result)
}
pub(crate) fn register(
    runtime: &mut Runtime,
    config: HttpConfig,
    policy: FilesystemPolicy,
) -> NativeResult<()> {
    let policy = policy.prepare()?;
    if config.timeout.is_zero()
        || Instant::now().checked_add(config.timeout).is_none()
        || config.max_response_bytes > i64::MAX as u64
    {
        return Err(NativeError::type_error(
            "invalid host HTTP timeout or response limit",
        ));
    }
    let jars = cookies::register(runtime)?;
    multipart::register(runtime)?;
    for operation in ["get", "post", "request", "download"] {
        let policy = policy.clone();
        let config = config.clone();
        let jars = jars.clone();
        let params = if operation == "get" {
            vec![NativeType::String]
        } else if operation == "post" {
            vec![
                NativeType::String,
                NativeType::OneOf(vec![NativeType::String, NativeType::Bytes]),
            ]
        } else {
            vec![NativeType::String, NativeType::String]
        };
        runtime.register(
            format!("Http::{operation}"),
            NativeFunction::new(
                NativeSignature::new(
                    params,
                    if operation == "download" {
                        NativeType::Map
                    } else {
                        NativeType::Object("HttpResponse")
                    },
                )
                .with_optional(vec![NativeType::OneOf(vec![
                    NativeType::List,
                    NativeType::Map,
                ])]),
                move |context, args| {
                    context.permissions.require("http.request")?;
                    let (method, url, option_index) = match operation {
                        "get" => ("GET", string(&args[0]), 1),
                        "post" => ("POST", string(&args[0]), 2),
                        "download" => ("GET", string(&args[0]), 2),
                        _ => (string(&args[0]), string(&args[1]), 2),
                    };
                    let mut options = options(args.get(option_index), &config)?;
                    if operation == "post" {
                        if let Some(Value::Map(entries)) = args.get(option_index) {
                            if entries
                                .iter()
                                .any(|(k, _)| *k == MapKey::String("body".into()))
                            {
                                return Err(NativeError::type_error(
                                    "post body must be supplied only as its second argument",
                                ));
                            }
                        }
                        options.body = match &args[1] {
                            Value::String(body) => Rc::from(body.as_bytes()),
                            Value::Bytes(body) => body.clone(),
                            _ => unreachable!(),
                        };
                    }
                    let destination = if operation == "download" {
                        if let Some(Value::Map(entries)) = args.get(option_index) {
                            if entries
                                .iter()
                                .any(|(key, _)| matches!(key, MapKey::String(key) if key == "body"))
                            {
                                return Err(NativeError::type_error(
                                    "download does not accept a request body",
                                ));
                            }
                        }
                        let path = paths::resolve(context, string(&args[1]))?;
                        check_destination(context, &policy, &path)?;
                        Some(path)
                    } else {
                        None
                    };
                    let uri: http::Uri = url
                        .parse()
                        .map_err(|_| NativeError::type_error("invalid HTTP URL"))?;
                    if !matches!(uri.scheme_str(), Some("http" | "https"))
                        || uri.host().is_none()
                        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                    {
                        return Err(NativeError::type_error(
                            "URL must be absolute HTTP/HTTPS without embedded credentials",
                        ));
                    }
                    let method = http::Method::from_bytes(method.as_bytes())
                        .map_err(|_| NativeError::type_error("invalid HTTP method"))?;
                    if method == http::Method::CONNECT {
                        return Err(NativeError::type_error(
                            "HTTP CONNECT tunnels are not supported",
                        ));
                    }
                    let jar = options
                        .jar
                        .as_ref()
                        .map(|value| jars.get(context, value))
                        .transpose()?;
                    if options
                        .headers
                        .iter()
                        .any(|(name, _)| name == http::header::COOKIE)
                        && jar.is_some()
                    {
                        return Err(NativeError::type_error(
                            "cookieJar cannot combine with a Cookie header",
                        ));
                    }
                    conveniences::validate_method(&method, options.retry.as_ref())?;
                    let started = Instant::now();
                    let enhanced = jar.is_some() || options.retry.is_some();
                    let deadline = if enhanced {
                        Some(started.checked_add(options.timeout).ok_or_else(|| {
                            NativeError::type_error("HTTP deadline exceeds platform range")
                        })?)
                    } else {
                        None
                    };
                    let mut response = if enhanced {
                        conveniences::send(context, &method, &uri, &options, jar, started)?
                    } else {
                        let mut builder = http::Request::builder().method(method).uri(uri.clone());
                        for (key, value) in &options.headers {
                            builder = builder.header(key, value);
                        }
                        let request = builder
                            .body(options.body.as_ref())
                            .map_err(|_| NativeError::type_error("invalid HTTP request"))?;
                        let agent: ureq::Agent = ureq::Agent::config_builder()
                            .http_status_as_error(false)
                            .timeout_global(Some(options.timeout))
                            .max_redirects(options.redirects)
                            .max_redirects_will_error(true)
                            .https_only(uri.scheme_str() == Some("https"))
                            .allow_non_standard_methods(true)
                            .proxy(None)
                            .build()
                            .into();
                        context.check_cancelled()?;
                        agent.run(request).map_err(failure)?
                    };
                    let status = response.status().as_u16();
                    let headers = response_headers(context, response.headers())?;
                    if let Some(destination) = destination {
                        if !(200..300).contains(&status) {
                            return Err(NativeError::new(
                                "HttpStatusError",
                                format!("download returned HTTP {status}"),
                            ));
                        }
                        return download(
                            context,
                            &policy,
                            &destination,
                            &mut response,
                            (options.limit, deadline),
                            status,
                            headers,
                        );
                    }
                    let mut size = size_of::<Value>();
                    grow(context, &mut size, header_bytes(&headers))?;
                    let mut reader = response
                        .body_mut()
                        .with_config()
                        .limit(options.limit)
                        .reader();
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 64 * 1024];
                    loop {
                        check_active(context, deadline)?;
                        let count = reader.read(&mut buffer).map_err(|e| failure(e.into()))?;
                        check_active(context, deadline)?;
                        if count == 0 {
                            break;
                        }
                        grow(context, &mut size, count)?;
                        bytes
                            .try_reserve(count)
                            .map_err(|_| resource("HTTP body allocation failed"))?;
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    // Freeze the transfer duration at body EOF, before text decoding
                    // or constructing the response object and script-visible values.
                    let duration_ms = crate::clock::milliseconds(started.elapsed());
                    context.check_cancelled()?;
                    if !options.binary {
                        decode(&bytes)?;
                    }
                    Ok(Value::NativeObject(Rc::new(Response {
                        status,
                        headers,
                        body: bytes.into(),
                        binary: options.binary,
                        duration_ms,
                    })))
                },
            ),
        )?;
    }
    Ok(())
}
fn check_active(context: &RuntimeContext<'_>, deadline: Option<Instant>) -> NativeResult<()> {
    context.check_cancelled()?;
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(NativeError::new(
            "TimeoutError",
            "HTTP total deadline exceeded",
        ));
    }
    Ok(())
}

fn response_headers(context: &RuntimeContext<'_>, source: &http::HeaderMap) -> NativeResult<Value> {
    let mut headers = Vec::new();
    let mut size = size_of::<Value>();
    for key in source.keys() {
        context.check_cancelled()?;
        grow(
            context,
            &mut size,
            size_of::<MapKey>() + key.as_str().len() + size_of::<Value>(),
        )?;
        let mut values = Vec::new();
        for value in source.get_all(key) {
            let value = value.to_str().map_err(|_| {
                NativeError::new("EncodingError", "HTTP response header is not ASCII text")
            })?;
            grow(context, &mut size, size_of::<Value>() + value.len())?;
            values.push(Value::String(value.into()));
        }
        headers.push((MapKey::String(key.as_str().into()), Value::list(values)));
    }
    Ok(Value::Map(headers))
}

fn string(value: &Value) -> &str {
    let Value::String(value) = value else {
        unreachable!()
    };
    value
}

fn check_destination(
    context: &RuntimeContext<'_>,
    policy: &FilesystemPolicy,
    path: &Path,
) -> NativeResult<()> {
    policy.check(context, path, true)?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(NativeError::new(
            "AlreadyExistsError",
            "download destination already exists",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(NativeError::new("FilesystemError", e.to_string())),
    }
}
fn download(
    context: &RuntimeContext<'_>,
    policy: &FilesystemPolicy,
    path: &Path,
    response: &mut ureq::http::Response<ureq::Body>,
    limits: (u64, Option<Instant>),
    status: u16,
    headers: Value,
) -> NativeResult {
    let (limit, deadline) = limits;
    check_active(context, deadline)?;
    let parent = path
        .parent()
        .ok_or_else(|| NativeError::new("PathError", "download destination needs a parent"))?;
    policy.check(context, parent, true)?;
    // Preflight the complete metadata result before creating or publishing a file.
    let result_size = size_of::<Value>()
        + header_bytes(&headers)
        + 2 * size_of::<Value>()
        + 3 * size_of::<MapKey>()
        + "status".len()
        + "headers".len()
        + "bytesWritten".len();
    budget(context, result_size)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".scripta-download-")
        .tempfile_in(parent)
        .map_err(|e| NativeError::new("FilesystemError", e.to_string()))?;
    let transfer: NativeResult<u64> = (|| {
        let mut reader = response.body_mut().with_config().limit(limit).reader();
        let mut buffer = [0; 64 * 1024];
        let mut count = 0u64;
        loop {
            check_active(context, deadline)?;
            let size = reader.read(&mut buffer).map_err(|e| failure(e.into()))?;
            check_active(context, deadline)?;
            if size == 0 {
                break;
            }
            check_active(context, deadline)?;
            temporary
                .write_all(&buffer[..size])
                .map_err(|e| NativeError::new("FilesystemError", e.to_string()))?;
            count += size as u64;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| NativeError::new("FilesystemError", e.to_string()))?;
        check_active(context, deadline)?;
        check_destination(context, policy, path)?;
        Ok(count)
    })();
    let count = match transfer {
        Ok(count) => count,
        Err(mut error) => {
            if let Err(cleanup) = temporary.close() {
                error
                    .message
                    .push_str(&format!("; download cleanup failed: {cleanup}"));
            }
            return Err(error);
        }
    };
    let result = Value::Map(vec![
        (
            MapKey::String("status".into()),
            Value::Int(i64::from(status)),
        ),
        (MapKey::String("headers".into()), headers),
        (
            MapKey::String("bytesWritten".into()),
            Value::Int(count as i64),
        ),
    ]);
    temporary
        .into_temp_path()
        .persist_noclobber(path)
        .map_err(|e| {
            let mut error = NativeError::new(
                if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                    "AlreadyExistsError"
                } else {
                    "FilesystemError"
                },
                e.error.to_string(),
            );
            if let Err(cleanup) = e.path.close() {
                error
                    .message
                    .push_str(&format!("; download cleanup failed: {cleanup}"));
            }
            error
        })?;
    Ok(result)
}

pub(crate) struct StreamResponse {
    pub reader: Box<dyn Read>,
    pub status: u16,
    pub headers: Value,
    pub limit: u64,
    pub deadline: Instant,
}
/// Streaming uses the same verified TLS, URI/header validation and redirect
/// policy as the buffered client. Source readers are deliberately not replayed.
pub(crate) fn open_stream(
    context: &RuntimeContext<'_>,
    config: &HttpConfig,
    method: &str,
    url: &str,
    value: &Value,
    body: Option<&mut dyn Read>,
) -> NativeResult<StreamResponse> {
    context.permissions.require("http.request")?;
    context.check_cancelled()?;
    let options = options(Some(value), config)?;
    if options.jar.is_some() || options.retry.is_some() {
        return Err(NativeError::type_error(
            "stream requests do not support cookieJar or retry options",
        ));
    }
    let uri: http::Uri = url
        .parse()
        .map_err(|_| NativeError::type_error("invalid HTTP URL"))?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.host().is_none()
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
    {
        return Err(NativeError::type_error(
            "URL must be absolute HTTP/HTTPS without embedded credentials",
        ));
    }
    let method = http::Method::from_bytes(method.as_bytes())
        .map_err(|_| NativeError::type_error("invalid HTTP method"))?;
    if method == http::Method::CONNECT {
        return Err(NativeError::type_error(
            "HTTP CONNECT tunnels are not supported",
        ));
    }
    if body.is_some() && options.redirects != 0 {
        if let Value::Map(entries) = value {
            if entries
                .iter()
                .any(|(k, _)| matches!(k,MapKey::String(k) if k=="maxRedirects"))
            {
                return Err(NativeError::type_error(
                    "stream uploads require maxRedirects=0; their source is not replayable",
                ));
            }
        }
    }
    let started = Instant::now();
    let deadline = started + options.timeout;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(options.timeout))
        .max_redirects(if body.is_some() { 0 } else { options.redirects })
        .max_redirects_will_error(true)
        .https_only(uri.scheme_str() == Some("https"))
        .allow_non_standard_methods(true)
        .proxy(None)
        .build()
        .into();
    let mut builder = http::Request::builder().method(method).uri(uri);
    for (key, value) in options.headers {
        builder = builder.header(key, value)
    }
    let result = if let Some(reader) = body {
        agent.run(
            builder
                .body(ureq::SendBody::from_reader(reader))
                .map_err(|_| NativeError::type_error("invalid HTTP request"))?,
        )
    } else {
        agent.run(
            builder
                .body(())
                .map_err(|_| NativeError::type_error("invalid HTTP request"))?,
        )
    };
    context.check_cancelled()?;
    let response = result.map_err(failure)?;
    let status = response.status().as_u16();
    let mut headers = vec![];
    let mut cost = size_of::<Value>();
    for key in response.headers().keys() {
        let mut values = vec![];
        grow(
            context,
            &mut cost,
            size_of::<MapKey>() + key.as_str().len() + size_of::<Value>(),
        )?;
        for value in response.headers().get_all(key) {
            let value = value.to_str().map_err(|_| {
                NativeError::new("EncodingError", "HTTP response header is not ASCII text")
            })?;
            grow(context, &mut cost, size_of::<Value>() + value.len())?;
            values.push(Value::String(value.into()));
        }
        headers.push((MapKey::String(key.as_str().into()), Value::list(values)));
    }
    grow(context, &mut cost, size_of::<Value>())?;
    let (_, body) = response.into_parts();
    Ok(StreamResponse {
        reader: Box::new(body.into_with_config().limit(options.limit).reader()),
        status,
        headers: Value::Map(headers),
        limit: options.limit,
        deadline,
    })
}

#[cfg(test)]
fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
    let mut settings = scr_runtime::ContextOptions::default();
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
