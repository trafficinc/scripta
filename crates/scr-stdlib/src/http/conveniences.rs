use super::*;
#[derive(Clone)]
pub(super) struct Retry {
    pub max: u32,
    backoff: Duration,
    maximum: Duration,
}
impl Retry {
    pub(super) fn parse(value: &Value) -> NativeResult<Self> {
        let Value::Map(entries) = value else {
            return Err(NativeError::type_error("retry must be a map"));
        };
        let mut result = Self {
            max: 0,
            backoff: Duration::from_millis(100),
            maximum: Duration::from_secs(30),
        };
        for (key, value) in entries {
            let (MapKey::String(key), Value::Int(count)) = (key, value) else {
                return Err(NativeError::type_error("invalid retry option"));
            };
            match key.as_str() {
                "maxRetries" if (0..=5).contains(count) => result.max = *count as u32,
                "backoffMs" if (0..=30000).contains(count) => {
                    result.backoff = Duration::from_millis(*count as u64)
                }
                "maxBackoffMs" if (0..=30000).contains(count) => {
                    result.maximum = Duration::from_millis(*count as u64)
                }
                _ => {
                    return Err(NativeError::type_error(
                        "unknown retry option or value outside its range",
                    ));
                }
            }
        }
        if result.backoff > result.maximum {
            return Err(NativeError::type_error(
                "retry backoffMs exceeds maxBackoffMs",
            ));
        }
        Ok(result)
    }
    fn delay(&self, attempt: u32, headers: &http::HeaderMap) -> Option<Duration> {
        let exponential = self.backoff.saturating_mul(1 << attempt).min(self.maximum);
        let retry_after = headers
            .get(http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|value| {
                if value.bytes().all(|b| b.is_ascii_digit()) && !value.is_empty() {
                    // Overflow means an unhonorably long delay, never immediate retry.
                    Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)))
                } else {
                    httpdate::parse_http_date(value).ok().map(|date| {
                        date.duration_since(std::time::SystemTime::now())
                            .unwrap_or_default()
                    })
                }
            })
            .unwrap_or_default();
        let delay = exponential.max(retry_after);
        (delay <= self.maximum).then_some(delay)
    }
}
pub(super) fn validate_method(method: &http::Method, retry: Option<&Retry>) -> NativeResult<()> {
    if retry.is_some_and(|r| r.max > 0) && !matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Err(NativeError::type_error(
            "positive retries require GET, HEAD, or OPTIONS",
        ));
    }
    Ok(())
}
fn timeout() -> NativeError {
    NativeError::new("TimeoutError", "HTTP total deadline exceeded")
}
fn remaining(context: &RuntimeContext<'_>, deadline: Instant) -> NativeResult<Duration> {
    context.check_cancelled()?;
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(timeout)
}
fn wait(context: &RuntimeContext<'_>, deadline: Instant, delay: Duration) -> NativeResult<()> {
    let until = Instant::now().checked_add(delay).ok_or_else(timeout)?;
    loop {
        remaining(context, deadline)?;
        let Some(left) = until.checked_duration_since(Instant::now()) else {
            return Ok(());
        };
        std::thread::sleep(left.min(Duration::from_millis(5)));
    }
}
fn discard(
    context: &RuntimeContext<'_>,
    response: &mut ureq::http::Response<ureq::Body>,
    limit: u64,
    deadline: Instant,
) -> NativeResult<()> {
    let mut reader = response.body_mut().with_config().limit(limit).reader();
    let mut buffer = [0; 16 * 1024];
    loop {
        remaining(context, deadline)?;
        if reader.read(&mut buffer).map_err(|e| failure(e.into()))? == 0 {
            return Ok(());
        }
    }
}
fn parsed_url(uri: &http::Uri) -> NativeResult<url::Url> {
    url::Url::parse(&uri.to_string()).map_err(|_| NativeError::type_error("invalid HTTP URL"))
}
fn target(base: &url::Url, location: &HeaderValue) -> NativeResult<url::Url> {
    let fail = || {
        NativeError::new(
            "RedirectError",
            "unsafe or unsupported HTTP redirect target",
        )
    };
    let mut next = base
        .join(location.to_str().map_err(|_| fail())?)
        .map_err(|_| fail())?;
    if !matches!(next.scheme(), "http" | "https")
        || next.host_str().is_none()
        || !next.username().is_empty()
        || next.password().is_some()
        || (base.scheme() == "https" && next.scheme() != "https")
    {
        return Err(fail());
    }
    next.set_fragment(None);
    Ok(next)
}
pub(super) fn send(
    context: &RuntimeContext<'_>,
    method: &http::Method,
    uri: &http::Uri,
    options: &Options,
    jar: Option<&cookies::Jar>,
    started: Instant,
) -> NativeResult<ureq::http::Response<ureq::Body>> {
    let deadline = started.checked_add(options.timeout).ok_or_else(timeout)?;
    let original = parsed_url(uri)?;
    let max_retries = options.retry.as_ref().map_or(0, |retry| retry.max);
    let mut hops = 0;
    for attempt in 0..=max_retries {
        let mut url = original.clone();
        let mut method = method.clone();
        let mut body = options.body.clone();
        let mut headers = options.headers.clone();
        loop {
            remaining(context, deadline)?;
            let mut request = http::Request::builder()
                .method(method.clone())
                .uri(url.as_str());
            for (key, value) in &headers {
                request = request.header(key, value);
            }
            if let Some(jar) = jar {
                if let Some(cookie) = jar.header(context, &url)? {
                    request = request.header(http::header::COOKIE, cookie);
                }
            }
            let request = request
                .body(body.as_ref())
                .map_err(|_| NativeError::type_error("invalid enhanced HTTP request"))?;
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(remaining(context, deadline)?))
                .max_redirects(0)
                .https_only(url.scheme() == "https")
                .allow_non_standard_methods(true)
                .proxy(None)
                .build()
                .into();
            context.check_cancelled()?;
            let response = agent.run(request).map_err(failure);
            remaining(context, deadline)?;
            let mut response = match response {
                Ok(response) => response,
                Err(error) if error.kind == "ConnectionError" && attempt < max_retries => {
                    let retry = options.retry.as_ref().unwrap();
                    let delay = retry
                        .delay(attempt, &http::HeaderMap::new())
                        .unwrap_or_default();
                    wait(context, deadline, delay)?;
                    break;
                }
                Err(error) => return Err(error),
            };
            if let Some(jar) = jar {
                jar.receive(context, &url, response.headers())?;
            }
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) && options.redirects != 0 {
                if hops >= options.redirects {
                    return Err(NativeError::new(
                        "RedirectError",
                        "HTTP redirect limit exceeded",
                    ));
                }
                let location = response
                    .headers()
                    .get(http::header::LOCATION)
                    .ok_or_else(|| {
                        NativeError::new("RedirectError", "missing redirect location")
                    })?;
                let next = target(&url, location)?;
                let change = (matches!(status, 301 | 302) && method == http::Method::POST)
                    || (status == 303 && method != http::Method::HEAD);
                if change {
                    method = http::Method::GET;
                    body = Rc::from([]);
                    headers.retain(|(name, _)| {
                        name != http::header::CONTENT_TYPE && name != http::header::CONTENT_ENCODING
                    });
                }
                headers.retain(|(name, _)| {
                    name != http::header::AUTHORIZATION && name != http::header::COOKIE
                });
                if url.origin() != next.origin() {
                    if !body.is_empty() {
                        return Err(NativeError::new(
                            "RedirectError",
                            "cross-origin body forwarding is not supported",
                        ));
                    }
                    headers.clear();
                }
                discard(context, &mut response, options.limit, deadline)?;
                hops += 1;
                url = next;
                continue;
            }
            if attempt < max_retries && matches!(status, 408 | 429 | 500 | 502 | 503 | 504) {
                let retry = options.retry.as_ref().unwrap();
                if let Some(delay) = retry.delay(attempt, response.headers()) {
                    if delay < remaining(context, deadline)? {
                        discard(context, &mut response, options.limit, deadline)?;
                        wait(context, deadline, delay)?;
                        break;
                    }
                }
            }
            return Ok(response);
        }
    }
    unreachable!("bounded retry loop always returns its final attempt")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_dates_bounds_and_unsafe_redirects_are_explicit() {
        let retry = Retry {
            max: 5,
            backoff: Duration::from_millis(10),
            maximum: Duration::from_secs(1),
        };
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::RETRY_AFTER,
            HeaderValue::from_static("Thu, 01 Jan 1970 00:00:00 GMT"),
        );
        assert_eq!(retry.delay(1, &headers), Some(Duration::from_millis(20)));
        headers.insert(http::header::RETRY_AFTER, HeaderValue::from_static("1"));
        assert_eq!(retry.delay(1, &headers), Some(Duration::from_secs(1)));
        headers.insert(http::header::RETRY_AFTER, HeaderValue::from_static("2"));
        assert!(retry.delay(1, &headers).is_none());
        let base = url::Url::parse("https://example.com/path").unwrap();
        for text in [
            "http://example.com/",
            "https://user:pass@example.com/",
            "file:///tmp/data",
        ] {
            assert_eq!(
                target(&base, &HeaderValue::from_str(text).unwrap())
                    .unwrap_err()
                    .kind,
                "RedirectError"
            );
        }
        assert_eq!(
            target(&base, &HeaderValue::from_static("../next"))
                .unwrap()
                .as_str(),
            "https://example.com/next"
        );
    }
}
