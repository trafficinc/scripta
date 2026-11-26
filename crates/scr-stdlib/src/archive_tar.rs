use super::*;
pub(super) fn encode(
    context: &RuntimeContext<'_>,
    entries: &[Source<'_>],
    options: &Options,
) -> NativeResult<Vec<u8>> {
    let io = Io::new(context);
    let mut archive = tar::Builder::new(Output::new(&io, options.output));
    for entry in entries {
        context.check_cancelled()?;
        let mut header = tar::Header::new_ustar();
        header
            .set_path(entry.path)
            .map_err(|_| invalid("TAR path cannot be represented in a USTAR header"))?;
        header.set_entry_type(if entry.data.is_some() {
            tar::EntryType::Regular
        } else {
            tar::EntryType::Directory
        });
        header.set_size(entry.data.map_or(0, <[u8]>::len) as u64);
        header.set_mode(if entry.data.is_some() { 0o644 } else { 0o755 });
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        io.result(
            archive.append(&header, Input::new(&io, entry.data.unwrap_or(&[]))),
            "cannot encode TAR entry",
        )?;
    }
    let output = io.result(archive.into_inner(), "cannot finish TAR archive")?;
    context.check_cancelled()?;
    Ok(output.bytes())
}
fn zeros(context: &RuntimeContext<'_>, bytes: &[u8]) -> NativeResult<()> {
    for chunk in bytes.chunks(CHUNK) {
        context.check_cancelled()?;
        if chunk.iter().any(|b| *b != 0) {
            return Err(invalid("nonzero TAR padding or trailing data"));
        }
    }
    Ok(())
}
fn terminated(field: &[u8]) -> NativeResult<()> {
    if let Some(end) = field.iter().position(|b| *b == 0) {
        if field[end..].iter().any(|b| *b != 0) {
            return Err(invalid("nonzero data after TAR path terminator"));
        }
    }
    Ok(())
}
pub(super) fn decode(
    context: &RuntimeContext<'_>,
    data: &[u8],
    options: &Options,
) -> NativeResult<Vec<Entry>> {
    let io = Io::new(context);
    let mut archive = tar::Archive::new(Input::new(&io, data));
    let entries = io
        .result(archive.entries(), "invalid TAR archive")?
        .raw(true);
    let mut result = Vec::new();
    let mut names = Names::default();
    let mut expanded = 0;
    let mut logical = size_of::<Value>();
    let mut offset = 0usize;
    for entry in entries {
        context.check_cancelled()?;
        let mut entry = io.result(entry, "invalid or truncated TAR header")?;
        if entry.raw_header_position() != offset as u64 {
            return Err(invalid("unsupported TAR record layout"));
        }
        let directory = match entry.header().entry_type() {
            tar::EntryType::Directory => true,
            tar::EntryType::Regular => false,
            _ => {
                return Err(invalid(
                    "TAR links, devices, sparse files and extension records are unsupported",
                ));
            }
        };
        terminated(&entry.header().as_bytes()[..100])?;
        if entry.header().as_ustar().is_some() {
            terminated(&entry.header().as_bytes()[345..500])?;
        }
        let path = entry.path_bytes();
        let path = std::str::from_utf8(&path).map_err(|_| invalid("TAR path is not UTF-8"))?;
        let path = name(path, directory, options)?.to_owned();
        let size = usize::try_from(entry.size())
            .map_err(|_| limit("TAR entry size exceeds address space"))?;
        if directory && size != 0 {
            return Err(invalid("TAR directory contains data"));
        }
        row_budget(context, &mut logical, &mut expanded, &path, size, options)?;
        names.add(context, &path, directory, options)?;
        let start = offset
            .checked_add(512)
            .ok_or_else(|| invalid("TAR offset overflow"))?;
        let end = start
            .checked_add(size)
            .ok_or_else(|| invalid("TAR size overflow"))?;
        let next = end
            .checked_add((512 - size % 512) % 512)
            .filter(|n| *n <= data.len())
            .ok_or_else(|| invalid("truncated TAR data or padding"))?;
        zeros(context, &data[end..next])?;
        let content = body(context, &io, &mut entry, size)?;
        result
            .try_reserve(1)
            .map_err(|_| limit("TAR entries allocation failed"))?;
        result.push(Entry {
            path,
            data: if directory { None } else { Some(content) },
        });
        offset = next;
    }
    let tail = data
        .get(offset..)
        .ok_or_else(|| invalid("TAR trailer offset overflow"))?;
    if tail.len() < 1024 || tail.len() % 512 != 0 {
        return Err(invalid("TAR requires a complete two-block end marker"));
    }
    zeros(context, tail)?;
    Ok(result)
}
