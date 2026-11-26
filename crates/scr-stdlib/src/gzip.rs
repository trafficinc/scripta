use super::*;
fn header(context: &RuntimeContext<'_>, data: &[u8]) -> NativeResult<()> {
    if data.len() < 10 || data[..3] != [0x1f, 0x8b, 8] || data[3] & 0xe0 != 0 {
        return Err(invalid("invalid gzip header"));
    }
    let flags = data[3];
    let mut cursor = 10usize;
    if flags & 4 != 0 {
        let len = data
            .get(cursor..cursor + 2)
            .ok_or_else(|| invalid("truncated gzip extra field"))?;
        cursor += 2 + usize::from(u16::from_le_bytes(len.try_into().unwrap()));
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            loop {
                context.check_cancelled()?;
                if cursor >= 65536 {
                    return Err(limit("gzip header exceeds 65536 bytes"));
                }
                let byte = *data
                    .get(cursor)
                    .ok_or_else(|| invalid("truncated gzip metadata"))?;
                cursor += 1;
                if byte == 0 {
                    break;
                }
            }
        }
    }
    if flags & 2 != 0 {
        cursor += 2;
    }
    if cursor > 65536 {
        return Err(limit("gzip header exceeds 65536 bytes"));
    }
    if cursor > data.len() {
        return Err(invalid("truncated gzip header"));
    }
    Ok(())
}
pub(super) fn invoke(
    context: &RuntimeContext<'_>,
    operation: &str,
    data: &[u8],
    options: &Options,
) -> NativeResult<Value> {
    options.input(data.len())?;
    bytes::payload(context, 0)?;
    let io = Io::new(context);
    let output = if operation == "compress" {
        let mut encoder = flate2::GzBuilder::new()
            .mtime(0)
            .operating_system(255)
            .write(
                Output::new(&io, options.output),
                flate2::Compression::default(),
            );
        for chunk in data.chunks(CHUNK) {
            context.check_cancelled()?;
            io.result(encoder.write_all(chunk), "cannot gzip-compress data")?;
        }
        io.result(encoder.finish(), "cannot finish gzip member")?
            .bytes()
    } else {
        let mut input = Input::new(&io, data);
        let mut result = Output::new(&io, options.output);
        let mut members = 0;
        while (input.cursor.position() as usize) < data.len() {
            context.check_cancelled()?;
            members += 1;
            if members > options.entries {
                return Err(limit("gzip exceeds maxMembers"));
            }
            header(context, &data[input.cursor.position() as usize..])?;
            let mut decoder = flate2::bufread::GzDecoder::new(&mut input);
            let mut buffer = [0; CHUNK];
            loop {
                context.check_cancelled()?;
                let n = io.result(
                    decoder.read(&mut buffer),
                    "invalid, truncated, or corrupt gzip member",
                )?;
                if n == 0 {
                    break;
                }
                io.result(result.write_all(&buffer[..n]), "gzip output limit exceeded")?;
            }
        }
        if members == 0 {
            return Err(invalid("gzip input contains no member"));
        }
        result.bytes()
    };
    context.check_cancelled()?;
    Ok(Value::Bytes(output.into()))
}
