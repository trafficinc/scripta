use super::*;
use std::{any::Any, cell::RefCell, rc::Weak, time::SystemTime};
const MAX_JARS: usize = 64;
const MAX_COOKIES: usize = 128;
const MAX_BYTES: usize = 64 * 1024;
fn closed() -> NativeError {
    NativeError::new("CookieError", "HTTP cookie jar is closed")
}
fn limit() -> NativeError {
    NativeError::new("CookieLimitError", "HTTP cookie jar limit exceeded")
}
#[derive(Clone)]
struct Cookie {
    origin: String,
    path: String,
    name: String,
    value: String,
    secure: bool,
    expires: Option<SystemTime>,
}
impl Cookie {
    fn bytes(&self) -> usize {
        size_of::<Self>() + self.origin.len() + self.path.len() + self.name.len() + self.value.len()
    }
}
#[derive(Default, Clone)]
struct State(Vec<Cookie>);
impl State {
    fn bytes(&self) -> usize {
        self.0.iter().map(Cookie::bytes).sum()
    }
}
pub(super) struct Jar {
    owner: Rc<()>,
    state: RefCell<Option<State>>,
}
impl std::fmt::Debug for Jar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpCookieJar")
    }
}
impl NativeObject for Jar {
    fn type_name(&self) -> &str {
        "HttpCookieJar"
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn payload_bytes(&self) -> usize {
        self.state.borrow().as_ref().map_or(0, State::bytes)
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.state.borrow().is_none())),
            _ => Err("unknown HttpCookieJar property".into()),
        }
    }
}
pub(super) struct Registry {
    owner: Rc<()>,
    runtime: Weak<()>,
    jars: RefCell<Vec<Weak<Jar>>>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        for jar in self.jars.get_mut().iter().filter_map(Weak::upgrade) {
            jar.state.borrow_mut().take();
        }
    }
}
impl Registry {
    pub(super) fn get<'a>(
        &self,
        context: &RuntimeContext<'_>,
        value: &'a Value,
    ) -> NativeResult<&'a Jar> {
        if !context
            .runtime_identity
            .as_ref()
            .is_some_and(|id| self.runtime.ptr_eq(&Rc::downgrade(id)))
        {
            return Err(NativeError::type_error(
                "cookie registry belongs to another runtime",
            ));
        }
        let Value::NativeObject(object) = value else {
            return Err(NativeError::type_error("expected HttpCookieJar"));
        };
        let jar = object
            .as_any()
            .and_then(|object| object.downcast_ref::<Jar>())
            .ok_or_else(|| NativeError::type_error("expected real HttpCookieJar"))?;
        if !Rc::ptr_eq(&self.owner, &jar.owner) {
            return Err(NativeError::type_error(
                "cookie jar belongs to another runtime or registration",
            ));
        }
        if jar.state.borrow().is_none() {
            return Err(closed());
        }
        Ok(jar)
    }
}
fn origin(url: &url::Url) -> String {
    url.origin().ascii_serialization()
}
fn valid_value(value: &str) -> bool {
    value.bytes().all(|b| {
        b == 0x21
            || (0x23..=0x2b).contains(&b)
            || (0x2d..=0x3a).contains(&b)
            || (0x3c..=0x5b).contains(&b)
            || (0x5d..=0x7e).contains(&b)
    })
}
fn parse(text: &str, url: &url::Url, now: SystemTime) -> Option<Cookie> {
    let mut parts = text.split(';');
    let (name, value) = parts.next()?.trim().split_once('=')?;
    if name.is_empty()
        || name.len() > 256
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    {
        return None;
    }
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    if !valid_value(value) {
        return None;
    }
    let default_path = url
        .path()
        .rsplit_once('/')
        .map(|(prefix, _)| if prefix.is_empty() { "/" } else { prefix })
        .unwrap_or("/");
    let mut cookie = Cookie {
        origin: origin(url),
        path: default_path.into(),
        name: name.into(),
        value: value.into(),
        secure: false,
        expires: None,
    };
    let mut max_age = None;
    let mut host_path = false;
    let mut seen = Vec::new();
    for attr in parts {
        let (key, val) = attr.trim().split_once('=').unwrap_or((attr.trim(), ""));
        let key = key.to_ascii_lowercase();
        if seen.contains(&key) {
            return None;
        }
        seen.push(key.clone());
        match key.as_str() {
            "domain" => return None,
            "path" if val.starts_with('/') && !val.bytes().any(|b| b < 0x20 || b == 0x7f) => {
                cookie.path = val.into();
                host_path = val == "/";
            }
            "secure" => cookie.secure = true,
            "max-age" => max_age = Some(val.parse::<i64>().ok()?),
            "expires" => {
                cookie.expires = httpdate::parse_http_date(val).ok().or_else(|| {
                    let date = chrono::DateTime::parse_from_rfc2822(val).ok()?;
                    (date.timestamp() < 0)
                        .then(|| {
                            SystemTime::UNIX_EPOCH
                                .checked_sub(Duration::from_secs(date.timestamp().unsigned_abs()))
                        })
                        .flatten()
                })
            }
            _ => {}
        }
    }
    if let Some(age) = max_age {
        cookie.expires = Some(if age <= 0 {
            SystemTime::UNIX_EPOCH
        } else {
            now.checked_add(Duration::from_secs(age as u64))?
        });
    }
    if cookie.secure && url.scheme() != "https" {
        return None;
    }
    if cookie.name.starts_with("__Secure-") && !cookie.secure {
        return None;
    }
    if cookie.name.starts_with("__Host-") && (!cookie.secure || cookie.path != "/" || !host_path) {
        return None;
    }
    Some(cookie)
}
impl Jar {
    pub(super) fn header(
        &self,
        context: &RuntimeContext<'_>,
        url: &url::Url,
    ) -> NativeResult<Option<HeaderValue>> {
        context.check_cancelled()?;
        let mut slot = self.state.borrow_mut();
        let state = slot.as_mut().ok_or_else(closed)?;
        let now = SystemTime::now();
        state
            .0
            .retain(|cookie| cookie.expires.is_none_or(|when| when > now));
        let origin = origin(url);
        let mut matches: Vec<_> = state
            .0
            .iter()
            .filter(|cookie| {
                cookie.origin == origin
                    && (!cookie.secure || url.scheme() == "https")
                    && (url.path() == cookie.path
                        || (url.path().starts_with(&cookie.path)
                            && (cookie.path.ends_with('/')
                                || url.path()[cookie.path.len()..].starts_with('/'))))
            })
            .collect();
        matches.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
        let length: usize = matches
            .iter()
            .map(|c| c.name.len() + c.value.len() + 3)
            .sum();
        crate::bytes::payload(context, length)?;
        let mut header = String::new();
        header
            .try_reserve(length)
            .map_err(|_| resource("cookie header allocation failed"))?;
        for cookie in matches {
            context.check_cancelled()?;
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&cookie.name);
            header.push('=');
            header.push_str(&cookie.value);
        }
        if header.is_empty() {
            return Ok(None);
        }
        Ok(Some(HeaderValue::from_str(&header).map_err(|_| {
            NativeError::new("CookieError", "invalid retained cookie")
        })?))
    }
    pub(super) fn receive(
        &self,
        context: &RuntimeContext<'_>,
        url: &url::Url,
        headers: &http::HeaderMap,
    ) -> NativeResult<()> {
        context.check_cancelled()?;
        let mut slot = self.state.borrow_mut();
        let state = slot.as_mut().ok_or_else(closed)?;
        crate::bytes::payload(context, state.bytes())?;
        let mut next = state.clone();
        let now = SystemTime::now();
        next.0.retain(|c| c.expires.is_none_or(|when| when > now));
        for value in headers.get_all(http::header::SET_COOKIE) {
            context.check_cancelled()?;
            if value.as_bytes().len() > 4096 {
                return Err(limit());
            }
            let Some(cookie) = value.to_str().ok().and_then(|text| parse(text, url, now)) else {
                continue;
            };
            let existing = next.0.iter().position(|c| {
                (c.origin.as_str(), c.path.as_str(), c.name.as_str())
                    == (
                        cookie.origin.as_str(),
                        cookie.path.as_str(),
                        cookie.name.as_str(),
                    )
            });
            if cookie.expires.is_some_and(|when| when <= now) {
                if let Some(index) = existing {
                    next.0.remove(index);
                }
                continue;
            }
            let size = next.bytes() - existing.map_or(0, |i| next.0[i].bytes()) + cookie.bytes();
            if size > MAX_BYTES || (existing.is_none() && next.0.len() >= MAX_COOKIES) {
                return Err(limit());
            }
            crate::bytes::payload(context, size)?;
            if let Some(index) = existing {
                next.0[index] = cookie;
            } else {
                next.0.push(cookie);
            }
        }
        context.check_cancelled()?;
        *state = next;
        Ok(())
    }
}
pub(super) fn register(runtime: &mut Runtime) -> NativeResult<Rc<Registry>> {
    let registry = Rc::new(Registry {
        owner: Rc::new(()),
        runtime: Rc::downgrade(&runtime.native_identity()),
        jars: RefCell::new(Vec::new()),
    });
    let create = registry.clone();
    runtime.register(
        "Http::cookieJar",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Object("HttpCookieJar")),
            move |context, _| {
                context.check_cancelled()?;
                crate::bytes::payload(context, 0)?;
                if !context
                    .runtime_identity
                    .as_ref()
                    .is_some_and(|id| create.runtime.ptr_eq(&Rc::downgrade(id)))
                {
                    return Err(NativeError::type_error(
                        "cookie registry belongs to another runtime",
                    ));
                }
                let mut jars = create.jars.borrow_mut();
                jars.retain(|jar| {
                    jar.upgrade()
                        .is_some_and(|jar| jar.state.borrow().is_some())
                });
                if jars.len() >= MAX_JARS {
                    return Err(limit());
                }
                let jar = Rc::new(Jar {
                    owner: create.owner.clone(),
                    state: RefCell::new(Some(State::default())),
                });
                jars.push(Rc::downgrade(&jar));
                Ok(Value::NativeObject(jar))
            },
        ),
    )?;
    let close = registry.clone();
    runtime.register(
        "Http::closeCookieJar",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("HttpCookieJar")], NativeType::Null),
            move |context, args| {
                context.check_cancelled()?;
                crate::bytes::payload(context, 0)?;
                // The ownership check is still required for an already-closed handle.
                let Value::NativeObject(object) = &args[0] else {
                    unreachable!()
                };
                let jar = object
                    .as_any()
                    .and_then(|o| o.downcast_ref::<Jar>())
                    .ok_or_else(|| NativeError::type_error("expected real HttpCookieJar"))?;
                if !Rc::ptr_eq(&jar.owner, &close.owner)
                    || !context
                        .runtime_identity
                        .as_ref()
                        .is_some_and(|id| close.runtime.ptr_eq(&Rc::downgrade(id)))
                {
                    return Err(NativeError::type_error(
                        "cookie jar belongs to another runtime or registration",
                    ));
                }
                jar.state.borrow_mut().take();
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_scope_secure_prefixes_expiry_and_path_rules_are_explicit() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let https = url::Url::parse("https://example.com/deep/page").unwrap();
        let http = url::Url::parse("http://example.com/deep/page").unwrap();
        assert_eq!(parse("a=x", &https, now).unwrap().path, "/deep");
        for text in [
            "a=x; Domain=example.com",
            "a=x; Domain=com",
            "__Host-a=x; Secure",
            "__Host-a=x; Secure; Path=bad",
            "__Secure-a=x",
            "a=x; Path=/; Path=/deep",
            "a=bad value",
        ] {
            assert!(parse(text, &https, now).is_none(), "{text}");
        }
        assert!(parse("a=x; Secure", &http, now).is_none());
        assert!(parse("__Host-a=x; Secure; Path=/", &https, now).is_some());
        let date = httpdate::fmt_http_date(now + Duration::from_secs(20));
        assert_eq!(
            parse(&format!("a=x; Expires={date}; Max-Age=0"), &https, now)
                .unwrap()
                .expires,
            Some(SystemTime::UNIX_EPOCH)
        );
        assert_eq!(
            parse(&format!("a=x; Expires={date}"), &https, now)
                .unwrap()
                .expires,
            Some(now + Duration::from_secs(20))
        );
    }
    #[test]
    fn cookie_count_storage_header_and_native_value_limits_are_exact_and_atomic() {
        super::super::with_context(|context| {
            let url = url::Url::parse("https://example.com/").unwrap();
            let jar = Jar {
                owner: Rc::new(()),
                state: RefCell::new(Some(State::default())),
            };
            let mut headers = http::HeaderMap::new();
            for i in 0..MAX_COOKIES {
                headers.append(
                    http::header::SET_COOKIE,
                    HeaderValue::from_str(&format!("c{i}=x")).unwrap(),
                );
            }
            jar.receive(context, &url, &headers).unwrap();
            assert_eq!(jar.state.borrow().as_ref().unwrap().0.len(), MAX_COOKIES);
            let before = jar.payload_bytes();
            headers.clear();
            headers.append(
                http::header::SET_COOKIE,
                HeaderValue::from_static("extra=x"),
            );
            assert_eq!(
                jar.receive(context, &url, &headers).unwrap_err().kind,
                "CookieLimitError"
            );
            assert_eq!(jar.payload_bytes(), before);
            jar.state.borrow_mut().replace(State::default());
            // Fill the logical ceiling, using bounded individual Set-Cookie fields.
            let mut index = 0;
            while jar.payload_bytes() < MAX_BYTES {
                let remaining = MAX_BYTES - jar.payload_bytes();
                let name = format!("c{index}");
                let overhead =
                    size_of::<Cookie>() + url.origin().ascii_serialization().len() + 1 + name.len();
                let value_len = (4096 - name.len() - 1).min(remaining - overhead);
                let text = format!("{name}={}", "x".repeat(value_len));
                headers.clear();
                headers.append(
                    http::header::SET_COOKIE,
                    HeaderValue::from_str(&text).unwrap(),
                );
                jar.receive(context, &url, &headers).unwrap();
                index += 1;
            }
            assert_eq!(jar.payload_bytes(), MAX_BYTES);
            let last = jar
                .state
                .borrow()
                .as_ref()
                .unwrap()
                .0
                .last()
                .unwrap()
                .clone();
            headers.clear();
            headers.append(
                http::header::SET_COOKIE,
                HeaderValue::from_str(&format!("{}={}x", last.name, last.value)).unwrap(),
            );
            assert_eq!(
                jar.receive(context, &url, &headers).unwrap_err().kind,
                "CookieLimitError"
            );
            assert_eq!(jar.payload_bytes(), MAX_BYTES);
            context.max_value_bytes = Some(size_of::<Value>() + MAX_BYTES - 1);
            assert_eq!(
                jar.receive(context, &url, &http::HeaderMap::new())
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            context.max_value_bytes = Some(size_of::<Value>() + MAX_BYTES);
            jar.receive(context, &url, &http::HeaderMap::new()).unwrap();
            context.cancellation = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            )));
            assert_eq!(
                jar.receive(context, &url, &headers).unwrap_err().kind,
                "Interrupted"
            );
        });
    }
}
