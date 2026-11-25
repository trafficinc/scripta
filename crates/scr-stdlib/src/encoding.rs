//! Strict binary-to-text codecs with bounded, interruptible backend calls.
use crate::bytes::{payload, resource};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};

// Multiples of three input bytes / four encoded bytes preserve group boundaries.
const RAW_CHUNK: usize = 3072;
const TEXT_CHUNK: usize = 4096;

#[derive(Clone, Copy)]
enum Codec {
    Base64,
    Base64Url,
    Hex,
}

fn invalid() -> NativeError {
    // Do not include the input: encoded data can contain credentials.
    NativeError::new("EncodingError", "invalid binary encoding")
}

fn encoded_len(codec: Codec, length: usize) -> NativeResult<usize> {
    let length = match codec {
        Codec::Hex => length.checked_mul(2),
        Codec::Base64 => length.div_ceil(3).checked_mul(4),
        Codec::Base64Url => (length / 3).checked_mul(4).and_then(|n| {
            n.checked_add(match length % 3 {
                0 => 0,
                1 => 2,
                _ => 3,
            })
        }),
    };
    length.ok_or_else(|| resource("encoded size overflow"))
}

fn decoded_len(codec: Codec, text: &[u8]) -> NativeResult<usize> {
    let length = text.len();
    match codec {
        Codec::Hex if length % 2 == 0 => Ok(length / 2),
        Codec::Base64 if length % 4 == 0 => {
            let padding = if text.ends_with(b"==") {
                2
            } else {
                usize::from(text.ends_with(b"="))
            };
            (length / 4 * 3).checked_sub(padding).ok_or_else(invalid)
        }
        Codec::Base64Url if length % 4 != 1 => Ok(length / 4 * 3 + length % 4 * 3 / 4),
        _ => Err(invalid()),
    }
}

fn encode(context: &RuntimeContext<'_>, codec: Codec, data: &[u8]) -> NativeResult<Value> {
    context.check_cancelled()?;
    let length = encoded_len(codec, data.len())?;
    payload(context, length)?;
    let mut result = String::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| resource("encoded string allocation failed"))?;
    for chunk in data.chunks(RAW_CHUNK) {
        context.check_cancelled()?;
        match codec {
            Codec::Base64 => STANDARD.encode_string(chunk, &mut result),
            Codec::Base64Url => URL_SAFE_NO_PAD.encode_string(chunk, &mut result),
            Codec::Hex => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                for byte in chunk {
                    result.push(char::from(HEX[(byte >> 4) as usize]));
                    result.push(char::from(HEX[(byte & 15) as usize]));
                }
            }
        }
    }
    Ok(Value::String(result))
}

fn nibble(byte: u8) -> NativeResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid()),
    }
}

fn decode(context: &RuntimeContext<'_>, codec: Codec, text: &str) -> NativeResult<Value> {
    context.check_cancelled()?;
    let input = text.as_bytes();
    let length = decoded_len(codec, input)?;
    payload(context, length)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| resource("decoded bytes allocation failed"))?;
    let mut buffer = [0; RAW_CHUNK];
    let mut remaining = input;
    while !remaining.is_empty() {
        context.check_cancelled()?;
        let count = remaining.len().min(TEXT_CHUNK);
        let (chunk, rest) = remaining.split_at(count);
        match codec {
            Codec::Hex => {
                for pair in chunk.chunks_exact(2) {
                    result.push(nibble(pair[0])? * 16 + nibble(pair[1])?);
                }
            }
            Codec::Base64 | Codec::Base64Url => {
                // Each backend call validates its own final group. Padding is
                // legal only at the end of the complete standard Base64 input.
                if !rest.is_empty() && chunk.contains(&b'=') {
                    return Err(invalid());
                }
                let engine = match codec {
                    Codec::Base64 => &STANDARD,
                    _ => &URL_SAFE_NO_PAD,
                };
                let written = engine
                    .decode_slice(chunk, &mut buffer)
                    .map_err(|_| invalid())?;
                result.extend_from_slice(&buffer[..written]);
            }
        }
        remaining = rest;
    }
    debug_assert_eq!(result.len(), length);
    Ok(Value::Bytes(result.into()))
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bytes, String as Text};
    for (name, codec, encoding) in [
        ("Base64::encode", Codec::Base64, true),
        ("Base64::decode", Codec::Base64, false),
        ("Base64::encodeUrl", Codec::Base64Url, true),
        ("Base64::decodeUrl", Codec::Base64Url, false),
        ("Hex::encode", Codec::Hex, true),
        ("Hex::decode", Codec::Hex, false),
    ] {
        let (input, output) = if encoding {
            (Bytes, Text)
        } else {
            (Text, Bytes)
        };
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(vec![input], output),
                move |context, args| match &args[0] {
                    Value::Bytes(data) if encoding => encode(context, codec, data),
                    Value::String(text) if !encoding => decode(context, codec, text),
                    _ => unreachable!(),
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        mem::size_of,
        sync::{Arc, atomic::AtomicBool},
    };

    #[test]
    fn exact_output_limits_and_cancellation_cover_every_codec() {
        let mut settings = ContextOptions::default();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        let mut context = RuntimeContext {
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
        };
        for codec in [Codec::Base64, Codec::Base64Url, Codec::Hex] {
            for data in [
                b"".as_slice(),
                b"f",
                b"fo",
                b"foo",
                &vec![0xff; RAW_CHUNK + 2],
            ] {
                context.max_value_bytes = None;
                let encoded = encode(&context, codec, data).unwrap();
                let Value::String(text) = &encoded else {
                    panic!("encoded string")
                };
                let required = size_of::<Value>() + text.len();
                context.max_value_bytes = Some(required);
                assert_eq!(encode(&context, codec, data).unwrap(), encoded);
                context.max_value_bytes = Some(required - 1);
                assert_eq!(
                    encode(&context, codec, data).unwrap_err().kind,
                    "ResourceLimitError"
                );
                let required = size_of::<Value>() + data.len();
                context.max_value_bytes = Some(required);
                assert_eq!(
                    decode(&context, codec, text).unwrap(),
                    Value::Bytes(data.into())
                );
                context.max_value_bytes = Some(required - 1);
                assert_eq!(
                    decode(&context, codec, text).unwrap_err().kind,
                    "ResourceLimitError"
                );
                context.max_value_bytes = None;
                context.cancellation = Some(Arc::new(AtomicBool::new(true)));
                assert_eq!(
                    encode(&context, codec, data).unwrap_err().kind,
                    "Interrupted"
                );
                assert_eq!(
                    decode(&context, codec, text).unwrap_err().kind,
                    "Interrupted"
                );
                context.cancellation = None;
            }
        }
    }

    #[test]
    fn size_arithmetic_rejects_overflow_without_allocating() {
        for codec in [Codec::Base64, Codec::Base64Url, Codec::Hex] {
            assert_eq!(
                encoded_len(codec, usize::MAX).unwrap_err().kind,
                "ResourceLimitError"
            );
        }
    }
}
