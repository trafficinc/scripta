use super::*;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct Meta {
    path: String,
    directory: bool,
    size: usize,
    start: usize,
    finish: usize,
    method: u16,
    crc: u32,
}
fn part(data: &[u8], offset: usize, size: usize) -> NativeResult<&[u8]> {
    data.get(
        offset
            ..offset
                .checked_add(size)
                .ok_or_else(|| invalid("ZIP offset overflow"))?,
    )
    .ok_or_else(|| invalid("truncated ZIP metadata"))
}
fn u16_at(data: &[u8], offset: usize) -> NativeResult<u16> {
    Ok(u16::from_le_bytes(
        part(data, offset, 2)?.try_into().unwrap(),
    ))
}
fn u32_at(data: &[u8], offset: usize) -> NativeResult<u32> {
    Ok(u32::from_le_bytes(
        part(data, offset, 4)?.try_into().unwrap(),
    ))
}
fn extra(context: &RuntimeContext<'_>, data: &[u8]) -> NativeResult<()> {
    let mut cursor = 0;
    while cursor < data.len() {
        context.check_cancelled()?;
        let tag = u16_at(data, cursor)?;
        let size = usize::from(u16_at(data, cursor + 2)?);
        if matches!(tag, 0x0001 | 0x7075 | 0x9901) {
            return Err(invalid(
                "ZIP64, Unicode-path overrides, and AES extra fields are unsupported",
            ));
        }
        part(data, cursor + 4, size)?;
        cursor += 4 + size;
    }
    Ok(())
}
// Read the bounded raw catalog directly. The third-party eager reader can recover
// from alternate EOCD candidates and deduplicate names before exposing its index;
// that would undermine preflight counts. It is deliberately not invoked here.
fn catalog(
    context: &RuntimeContext<'_>,
    data: &[u8],
    options: &Options,
) -> NativeResult<Vec<Meta>> {
    if data.len() < 22 {
        return Err(invalid("missing ZIP end record"));
    }
    let first = data.len().saturating_sub(22 + 65535);
    let mut end = None;
    for offset in (first..=data.len() - 22).rev() {
        if offset % 1024 == 0 {
            context.check_cancelled()?;
        }
        if part(data, offset, 4)? == b"PK\x05\x06"
            && offset + 22 + usize::from(u16_at(data, offset + 20)?) == data.len()
        {
            end = Some(offset);
            break;
        }
    }
    let end = end.ok_or_else(|| invalid("missing or truncated ZIP end record"))?;
    let count = usize::from(u16_at(data, end + 10)?);
    if u16_at(data, end + 4)? != 0
        || u16_at(data, end + 6)? != 0
        || u16_at(data, end + 8)? as usize != count
        || count == 65535
    {
        return Err(invalid("multi-disk and ZIP64 archives are unsupported"));
    }
    if count > options.entries {
        return Err(limit("ZIP catalog exceeds maxEntries"));
    }
    let central = u32_at(data, end + 16)? as usize;
    if central.checked_add(u32_at(data, end + 12)? as usize) != Some(end) {
        return Err(invalid("invalid ZIP central directory range"));
    }
    let mut offset = central;
    let mut names = Names::default();
    let mut result = Vec::new();
    let mut ranges = Vec::new();
    let mut logical = size_of::<Value>();
    let mut expanded = 0;
    for _ in 0..count {
        context.check_cancelled()?;
        let header = part(data, offset, 46)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(invalid("invalid ZIP central header"));
        }
        let flags = u16_at(header, 8)?;
        let method = u16_at(header, 10)?;
        if u16_at(header, 6)? > 20
            || flags & !0x0806 != 0
            || !matches!(method, 0 | 8)
            || (method == 0 && flags & 6 != 0)
        {
            return Err(invalid(
                "unsupported ZIP version, flags, encryption, descriptor, or compression",
            ));
        }
        let compressed = u32_at(header, 20)? as usize;
        let size = u32_at(header, 24)? as usize;
        if compressed == u32::MAX as usize || size == u32::MAX as usize || u16_at(header, 34)? != 0
        {
            return Err(invalid("ZIP64 and multi-disk entries are unsupported"));
        }
        let name_size = usize::from(u16_at(header, 28)?);
        let extra_size = usize::from(u16_at(header, 30)?);
        let comment = usize::from(u16_at(header, 32)?);
        let raw = part(data, offset + 46, name_size)?;
        if flags & 0x800 == 0 && !raw.is_ascii() {
            return Err(invalid("non-ASCII ZIP names require UTF-8 encoding flag"));
        }
        let raw_name = std::str::from_utf8(raw).map_err(|_| invalid("ZIP path is not UTF-8"))?;
        let directory = raw_name.ends_with('/');
        let path = name(raw_name, directory, options)?;
        let attributes = u32_at(header, 38)?;
        // Refuse special Unix-type bits even under a misleading creator OS.
        let mode = attributes >> 16;
        let kind = mode & 0o170000;
        if !matches!(kind, 0 | 0o100000 | 0o040000)
            || (kind == 0o040000 && !directory)
            || (kind == 0o100000 && directory)
            || attributes & 0x400 != 0
            || (attributes & 0x10 != 0 && !directory)
        {
            return Err(invalid("unsupported or inconsistent ZIP entry type"));
        }
        if directory && size != 0 {
            return Err(invalid("ZIP directory contains data"));
        }
        row_budget(context, &mut logical, &mut expanded, path, size, options)?;
        names.add(context, path, directory, options)?;
        extra(context, part(data, offset + 46 + name_size, extra_size)?)?;
        let next = offset + 46 + name_size + extra_size + comment;
        if next > end {
            return Err(invalid("ZIP catalog extends beyond its directory"));
        }
        let local = u32_at(header, 42)? as usize;
        let local_header = part(data, local, 30)?;
        if &local_header[..4] != b"PK\x03\x04"
            || u16_at(local_header, 4)? > 20
            || u16_at(local_header, 6)? != flags
            || u16_at(local_header, 8)? != method
            || u32_at(local_header, 14)? != u32_at(header, 16)?
            || u32_at(local_header, 18)? as usize != compressed
            || u32_at(local_header, 22)? as usize != size
        {
            return Err(invalid("inconsistent ZIP local and central headers"));
        }
        let local_name = usize::from(u16_at(local_header, 26)?);
        let local_extra = usize::from(u16_at(local_header, 28)?);
        if part(data, local + 30, local_name)? != raw {
            return Err(invalid("ZIP local name differs from central name"));
        }
        extra(context, part(data, local + 30 + local_name, local_extra)?)?;
        let content = local + 30 + local_name + local_extra;
        let finish = content
            .checked_add(compressed)
            .filter(|n| *n <= central)
            .ok_or_else(|| invalid("ZIP data overlaps its central directory"))?;
        ranges.push((local, finish));
        result
            .try_reserve(1)
            .map_err(|_| limit("ZIP catalog allocation failed"))?;
        result.push(Meta {
            path: path.into(),
            directory,
            size,
            start: content,
            finish,
            method,
            crc: u32_at(header, 16)?,
        });
        offset = next;
    }
    if offset != end {
        return Err(invalid("ZIP catalog count does not match directory size"));
    }
    ranges.sort_unstable();
    let mut position = 0;
    for (start, end) in ranges {
        context.check_cancelled()?;
        if start != position {
            return Err(invalid(
                "overlapping ZIP records, prefixes, or record gaps are unsupported",
            ));
        }
        position = end;
    }
    if position != central {
        return Err(invalid("ZIP local records do not cover the data area"));
    }
    Ok(result)
}
pub(super) fn encode(
    context: &RuntimeContext<'_>,
    entries: &[Source<'_>],
    options: &Options,
) -> NativeResult<Vec<u8>> {
    let io = Io::new(context);
    let mut archive = ZipWriter::new(Output::new(&io, options.output));
    let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
        .map_err(|_| invalid("invalid ZIP timestamp"))?;
    for entry in entries {
        context.check_cancelled()?;
        let directory = entry.data.is_none();
        let method = if options.stored || directory {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        let file_options = SimpleFileOptions::default()
            .compression_method(method)
            .last_modified_time(timestamp)
            .unix_permissions(if directory { 0o755 } else { 0o644 });
        if let Some(data) = entry.data {
            io.result(
                archive.start_file(entry.path, file_options),
                "cannot encode ZIP entry",
            )?;
            for chunk in data.chunks(CHUNK) {
                context.check_cancelled()?;
                io.result(archive.write_all(chunk), "cannot encode ZIP data")?;
            }
        } else {
            io.result(
                archive.add_directory(entry.path, file_options),
                "cannot encode ZIP directory",
            )?;
        }
    }
    let output = io.result(archive.finish(), "cannot finish ZIP archive")?;
    context.check_cancelled()?;
    Ok(output.bytes())
}
fn inflate(context: &RuntimeContext<'_>, data: &[u8], expected: usize) -> NativeResult<Vec<u8>> {
    let mut decoder = flate2::Decompress::new(false);
    let mut result = Vec::new();
    result
        .try_reserve_exact(expected)
        .map_err(|_| limit("ZIP expansion allocation failed"))?;
    let mut consumed = 0usize;
    let mut buffer = [0; CHUNK];
    loop {
        context.check_cancelled()?;
        let end = data.len().min(consumed + CHUNK);
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        // Finish assumes the remaining output fits. Signal EOF only when that
        // is true for the advertised size; never require an unbounded buffer.
        let flush = if end == data.len() && expected - result.len() <= CHUNK {
            flate2::FlushDecompress::Finish
        } else {
            flate2::FlushDecompress::None
        };
        let status = decoder.decompress(&data[consumed..end], &mut buffer, flush);
        context.check_cancelled()?;
        let status = status.map_err(|_| invalid("corrupt ZIP DEFLATE stream"))?;
        let used = (decoder.total_in() - before_in) as usize;
        let count = (decoder.total_out() - before_out) as usize;
        consumed += used;
        if result.len() + count > expected {
            return Err(invalid("ZIP entry exceeds its declared size"));
        }
        result.extend_from_slice(&buffer[..count]);
        if status == flate2::Status::StreamEnd {
            if consumed != data.len() || result.len() != expected {
                return Err(invalid(
                    "ZIP DEFLATE size mismatch or trailing compressed data",
                ));
            }
            return Ok(result);
        }
        if used == 0 && count == 0 {
            return Err(invalid("truncated ZIP DEFLATE stream"));
        }
    }
}
pub(super) fn decode(
    context: &RuntimeContext<'_>,
    data: &[u8],
    options: &Options,
) -> NativeResult<Vec<Entry>> {
    let metadata = catalog(context, data, options)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(metadata.len())
        .map_err(|_| limit("ZIP entries allocation failed"))?;
    for meta in metadata {
        context.check_cancelled()?;
        let compressed = &data[meta.start..meta.finish];
        let content = if meta.method == 0 {
            if compressed.len() != meta.size {
                return Err(invalid("stored ZIP entry size mismatch"));
            }
            let io = Io::new(context);
            body(context, &io, &mut Input::new(&io, compressed), meta.size)?
        } else {
            inflate(context, compressed, meta.size)?
        };
        let mut crc = crc32fast::Hasher::new();
        for chunk in content.chunks(CHUNK) {
            context.check_cancelled()?;
            crc.update(chunk);
        }
        if crc.finalize() != meta.crc {
            return Err(invalid("ZIP entry CRC mismatch"));
        }
        result.push(Entry {
            path: meta.path,
            data: if meta.directory { None } else { Some(content) },
        });
    }
    Ok(result)
}
