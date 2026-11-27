use super::*;

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PARTS: usize = 128;
fn invalid() -> NativeError {
    NativeError::new(
        "MultipartError",
        "invalid multipart part or unsafe metadata",
    )
}
fn metadata(text: &str, filename: bool) -> NativeResult<()> {
    if text.is_empty()
        || text.len() > 256
        || !text
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) && !b"\"\\".contains(&b))
        || (filename && (matches!(text, "." | "..") || text.contains(['/', ':'])))
    {
        return Err(invalid());
    }
    Ok(())
}
struct Part<'a> {
    header: String,
    data: &'a [u8],
}
fn encode(
    context: &RuntimeContext<'_>,
    parts: &Value,
    options: Option<&Value>,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    let mut limit = MAX_BYTES;
    let mut part_limit = MAX_PARTS;
    if let Some(value) = options {
        let Value::Map(entries) = value else {
            return Err(NativeError::type_error("multipart options must be a map"));
        };
        for (key, value) in entries {
            let (MapKey::String(key), Value::Int(count)) = (key, value) else {
                return Err(NativeError::type_error("invalid multipart option"));
            };
            let target = match key.as_str() {
                "maxOutputBytes" => &mut limit,
                "maxParts" => &mut part_limit,
                _ => return Err(NativeError::type_error("unknown multipart option")),
            };
            let maximum = if key == "maxParts" {
                MAX_PARTS
            } else {
                MAX_BYTES
            };
            *target = usize::try_from(*count)
                .ok()
                .filter(|n| *n <= maximum)
                .ok_or_else(|| {
                    NativeError::new("RangeError", "multipart option exceeds hard range")
                })?;
        }
    }
    let Value::List(parts) = parts else {
        unreachable!()
    };
    if parts.len() > part_limit {
        return Err(resource("multipart part count exceeded"));
    }
    let mut parsed = Vec::new();
    let mut payload_size = 0usize;
    for part in parts {
        context.check_cancelled()?;
        let Value::Map(entries) = part else {
            return Err(invalid());
        };
        let (mut name, mut data, mut filename, mut content_type) = (None, None, None, None);
        for (key, value) in entries {
            let MapKey::String(key) = key else {
                return Err(invalid());
            };
            match (key.as_str(), value) {
                ("name", Value::String(text)) => {
                    metadata(text, false)?;
                    name = Some(text.as_str());
                }
                ("data", Value::String(text)) => data = Some(text.as_bytes()),
                ("data", Value::Bytes(bytes)) => data = Some(bytes.as_ref()),
                ("filename", Value::String(text)) => {
                    metadata(text, true)?;
                    filename = Some(text.as_str());
                }
                ("contentType", Value::String(text)) => {
                    metadata(text, false)?;
                    let token = |s: &str| {
                        !s.is_empty()
                            && s.bytes().all(|b| {
                                b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
                            })
                    };
                    if !text
                        .split_once('/')
                        .is_some_and(|(left, right)| token(left) && token(right))
                    {
                        return Err(invalid());
                    }
                    content_type = Some(text.as_str());
                }
                _ => return Err(invalid()),
            }
        }
        let name = name.ok_or_else(invalid)?;
        let data = data.ok_or_else(invalid)?;
        if content_type.is_some() && filename.is_none() {
            return Err(invalid());
        }
        payload_size = payload_size
            .checked_add(data.len())
            .filter(|n| *n <= limit)
            .ok_or_else(|| resource("multipart output exceeds limit"))?;
        let mut header = format!("Content-Disposition: form-data; name=\"{name}\"");
        if let Some(filename) = filename {
            header.push_str(&format!(
                "; filename=\"{filename}\"\r\nContent-Type: {}",
                content_type.unwrap_or("application/octet-stream")
            ));
        }
        header.push_str("\r\n\r\n");
        parsed.push(Part { header, data });
    }
    // OS entropy plus explicit collision rejection; never use a predictable fallback.
    let mut boundary = String::new();
    for _ in 0..8 {
        let mut random = [0u8; 24];
        getrandom::fill(&mut random)
            .map_err(|_| NativeError::new("MultipartError", "multipart boundary entropy failed"))?;
        boundary = "scripta-".to_owned();
        for byte in random {
            use std::fmt::Write;
            write!(&mut boundary, "{byte:02x}").unwrap();
        }
        let mut collision = false;
        for part in &parsed {
            for offset in (0..part.data.len()).step_by(16 * 1024) {
                context.check_cancelled()?;
                let end = part.data.len().min(offset + 16 * 1024 + boundary.len());
                collision |= part.data[offset..end]
                    .windows(boundary.len())
                    .any(|w| w == boundary.as_bytes());
            }
        }
        if !collision {
            break;
        }
        boundary.clear();
    }
    if boundary.is_empty() {
        return Err(NativeError::new(
            "MultipartError",
            "could not generate collision-free boundary",
        ));
    }
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let mut wire_size = boundary.len() + 6;
    for part in &parsed {
        wire_size = wire_size
            .checked_add(boundary.len() + 6 + part.header.len() + part.data.len())
            .filter(|n| *n <= limit)
            .ok_or_else(|| resource("multipart output exceeds limit"))?;
    }
    if wire_size > limit {
        return Err(resource("multipart output exceeds limit"));
    }
    let mut total = 3 * size_of::<Value>()
        + 2 * MapKey::LOGICAL_BYTES
        + "contentType".len()
        + "body".len()
        + content_type.len();
    grow(context, &mut total, wire_size)?;
    let mut body = Vec::new();
    body.try_reserve_exact(wire_size)
        .map_err(|_| resource("multipart allocation failed"))?;
    for part in parsed {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(part.header.as_bytes());
        for chunk in part.data.chunks(16 * 1024) {
            context.check_cancelled()?;
            body.extend_from_slice(chunk);
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    context.check_cancelled()?;
    Ok(Value::map(vec![
        (
            MapKey::String("contentType".into()),
            Value::String(content_type),
        ),
        (MapKey::String("body".into()), Value::Bytes(body.into())),
    ]))
}
pub(super) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Http::multipart",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::List], NativeType::Map)
                .with_optional(vec![NativeType::Map]),
            |context, args| encode(context, &args[0], args.get(1)),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multipart_preflights_exact_result_budget_and_native_cancellation() {
        super::super::with_context(|context| {
            let parts = Value::list(vec![Value::map(vec![
                (MapKey::String("name".into()), Value::String("field".into())),
                (
                    MapKey::String("data".into()),
                    Value::Bytes(vec![0xff; 32769].into()),
                ),
            ])]);
            let Value::Map(result) = encode(context, &parts, None).unwrap() else {
                panic!()
            };
            let Value::String(kind) = &result[0].1 else {
                panic!()
            };
            let Value::Bytes(body) = &result[1].1 else {
                panic!()
            };
            let needed = 3 * size_of::<Value>()
                + 2 * MapKey::LOGICAL_BYTES
                + "contentType".len()
                + "body".len()
                + kind.len()
                + body.len();
            context.max_value_bytes = Some(needed - 1);
            assert_eq!(
                encode(context, &parts, None).unwrap_err().kind,
                "ResourceLimitError"
            );
            context.max_value_bytes = Some(needed);
            assert!(encode(context, &parts, None).is_ok());
            context.cancellation = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            )));
            assert_eq!(
                encode(context, &parts, None).unwrap_err().kind,
                "Interrupted"
            );
        });
    }
}
