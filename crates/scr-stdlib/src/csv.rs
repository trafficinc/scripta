//! Bounded CSV records: physical lines are never treated as complete records.
use crate::{
    bytes::{budget, grow, payload, resource},
    filesystem::{FilesystemPolicy, open_read},
    paths,
};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType,
    Runtime, RuntimeContext, Value,
};
use std::{
    cell::RefCell,
    fs::File,
    io::{self, BufRead, BufReader, Cursor, Read, Write},
    mem::size_of,
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
struct Options {
    delimiter: u8,
    limit: usize,
}
fn options(value: Option<&Value>) -> NativeResult<Options> {
    let mut result = Options {
        delimiter: b',',
        limit: 1024 * 1024,
    };
    let entries = match value {
        None => return Ok(result),
        Some(Value::List(v)) if v.is_empty() => return Ok(result),
        Some(Value::Map(entries)) => entries,
        _ => {
            return Err(NativeError::type_error(
                "CSV options require a map or empty list",
            ));
        }
    };
    for (key, value) in entries {
        match (key, value) {
            (MapKey::String(key), Value::String(v))
                if key == "delimiter"
                    && v.len() == 1
                    && v.is_ascii()
                    && !matches!(v.as_bytes()[0], b'"' | b'\r' | b'\n' | 0) =>
            {
                result.delimiter = v.as_bytes()[0]
            }
            (MapKey::String(key), Value::Int(n))
                if key == "maxRecordBytes" && (1..=16 * 1024 * 1024).contains(n) =>
            {
                result.limit = *n as usize
            }
            _ => {
                return Err(NativeError::type_error(
                    "unknown CSV option or invalid value",
                ));
            }
        }
    }
    Ok(result)
}
fn error(record: usize, message: impl std::fmt::Display) -> NativeError {
    NativeError::new("CsvError", format!("CSV record {record}: {message}"))
}
#[derive(Debug)]
struct Records<R> {
    input: R,
    options: Options,
    width: Option<usize>,
    record: usize,
}
impl<R: BufRead> Records<R> {
    fn next(&mut self, context: &RuntimeContext<'_>) -> NativeResult<Option<Value>> {
        self.record += 1;
        // 0: field start, 1: unquoted, 2: quoted, 3: closing/escaped quote.
        let mut state = 0;
        let mut raw = Vec::new();
        let mut bom_checked = self.record != 1;
        let mut bom_bytes = 0;
        let mut field = 1;
        loop {
            context.check_cancelled()?;
            let available = self.input.fill_buf().map_err(|e| error(self.record, e))?;
            let Some(&byte) = available.first() else {
                if state == 2 {
                    return Err(error(
                        self.record,
                        format!("field {field}: unterminated quoted field"),
                    ));
                }
                if raw.is_empty() {
                    return Ok(None);
                }
                break;
            };
            if raw.len() + bom_bytes >= self.options.limit {
                return Err(resource("CSV record exceeds maxRecordBytes"));
            }
            payload(context, raw.len() + 1)?;
            raw.try_reserve(1)
                .map_err(|_| resource("CSV record allocation failed"))?;
            raw.push(byte);
            self.input.consume(1);
            if !bom_checked {
                if !b"\xef\xbb\xbf".starts_with(&raw) {
                    bom_checked = true;
                } else if raw.len() == 3 {
                    bom_checked = true;
                    bom_bytes = 3;
                    raw.clear();
                    state = 0;
                    continue;
                }
            }
            if state != 2 && matches!(byte, b'\r' | b'\n') {
                if byte == b'\r'
                    && self
                        .input
                        .fill_buf()
                        .map_err(|e| error(self.record, e))?
                        .first()
                        == Some(&b'\n')
                {
                    if raw.len() + bom_bytes >= self.options.limit {
                        return Err(resource("CSV record exceeds maxRecordBytes"));
                    }
                    payload(context, raw.len() + 1)?;
                    raw.try_reserve(1)
                        .map_err(|_| resource("CSV record allocation failed"))?;
                    raw.push(b'\n');
                    self.input.consume(1);
                }
                break;
            }
            state = match (state, byte) {
                (0, b'"') => 2,
                (2, b'"') => 3,
                (2, _) => 2,
                (3, b'"') => 2,
                (0 | 1 | 3, b) if b == self.options.delimiter => {
                    field += 1;
                    0
                }
                (3, _) => {
                    return Err(error(
                        self.record,
                        format!("field {field}: unexpected character after closing quote"),
                    ));
                }
                (1, b'"') => {
                    return Err(error(
                        self.record,
                        format!("field {field}: quote inside unquoted field"),
                    ));
                }
                _ => 1,
            };
        }
        // Prefix a synthetic empty field so csv never strips a field's BOM or
        // omits a blank record. Only the scanner strips the document's initial BOM.
        let prefix = [self.options.delimiter];
        let input = prefix.as_slice().chain(raw.as_slice());
        let mut parser = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(self.options.delimiter)
            .from_reader(input);
        let mut record = csv::StringRecord::new();
        parser
            .read_record(&mut record)
            .map_err(|e| error(self.record, e))?;
        let count = record.len() - 1;
        if self.width.is_some_and(|width| width != count) {
            return Err(error(
                self.record,
                format!("expected {} fields, got {count}", self.width.unwrap()),
            ));
        }
        self.width = Some(count);
        let mut size = size_of::<Value>();
        budget(context, size)?;
        let mut values = Vec::new();
        for field in record.iter().skip(1) {
            grow(context, &mut size, size_of::<Value>() + field.len())?;
            values
                .try_reserve(1)
                .map_err(|_| resource("CSV field allocation failed"))?;
            values.push(Value::String(field.into()));
        }
        Ok(Some(Value::List(values)))
    }
}
#[derive(Debug)]
struct Reader {
    state: RefCell<Option<Records<BufReader<File>>>>,
    owner: Rc<()>,
}
impl NativeObject for Reader {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "CsvReader"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.state.borrow().is_none())),
            _ => Err(format!("unknown CsvReader property {name}")),
        }
    }
}
fn reader(value: &Value) -> NativeResult<&Reader> {
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("expected CsvReader"));
    };
    object
        .as_any()
        .and_then(|o| o.downcast_ref())
        .ok_or_else(|| NativeError::type_error("expected real CsvReader"))
}
struct Output {
    data: Vec<u8>,
    remaining: usize,
    record_remaining: usize,
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining || bytes.len() > self.record_remaining {
            return Err(io::Error::other(
                "CSV output exceeds value or record byte limit",
            ));
        }
        self.data
            .try_reserve(bytes.len())
            .map_err(|_| io::Error::other("CSV allocation failed"))?;
        self.data.extend_from_slice(bytes);
        self.remaining -= bytes.len();
        self.record_remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn register(runtime: &mut Runtime, policy: FilesystemPolicy) -> NativeResult<()> {
    use NativeType::{Any, List, Map, Null, Object, String as Text};
    let optional = vec![NativeType::OneOf(vec![Map, List])];
    runtime.register(
        "Csv::parse",
        NativeFunction::new(
            NativeSignature::new(vec![Text], List).with_optional(optional.clone()),
            |context, args| {
                let Value::String(text) = &args[0] else {
                    unreachable!()
                };
                let mut reader = Records {
                    input: Cursor::new(text.as_bytes()),
                    options: options(args.get(1))?,
                    width: None,
                    record: 0,
                };
                let mut rows = Vec::new();
                let mut total = size_of::<Value>();
                budget(context, total)?;
                while let Some(row) = reader.next(context)? {
                    let Value::List(fields) = &row else {
                        unreachable!()
                    };
                    grow(context, &mut total, size_of::<Value>())?;
                    for field in fields {
                        let Value::String(field) = field else {
                            unreachable!()
                        };
                        grow(context, &mut total, size_of::<Value>() + field.len())?;
                    }
                    rows.try_reserve(1)
                        .map_err(|_| resource("CSV row allocation failed"))?;
                    rows.push(row);
                }
                Ok(Value::List(rows))
            },
        ),
    )?;
    runtime.register(
        "Csv::encode",
        NativeFunction::new(
            NativeSignature::new(vec![List], Text).with_optional(optional.clone()),
            |context, args| {
                let Value::List(rows) = &args[0] else {
                    unreachable!()
                };
                let options = options(args.get(1))?;
                payload(context, 0)?;
                let mut output = Output {
                    data: Vec::new(),
                    remaining: context
                        .max_value_bytes
                        .map_or(usize::MAX, |n| n - size_of::<Value>()),
                    record_remaining: options.limit,
                };
                let mut width = None;
                for (index, row) in rows.iter().enumerate() {
                    context.check_cancelled()?;
                    let Value::List(fields) = row else {
                        return Err(NativeError::type_error("CSV rows must be lists of strings"));
                    };
                    if fields.is_empty() || width.is_some_and(|n| n != fields.len()) {
                        return Err(error(
                            index + 1,
                            "rows must have a consistent, nonzero field count",
                        ));
                    }
                    width = Some(fields.len());
                    if fields.iter().any(|v| !matches!(v, Value::String(_))) {
                        return Err(NativeError::type_error("CSV fields must be strings"));
                    }
                    output.record_remaining = options.limit;
                    // A BOM in the first field is data, not a document marker.
                    let protect_bom = index == 0
                        && matches!(&fields[0], Value::String(s) if s.starts_with('\u{feff}'));
                    let mut writer = csv::WriterBuilder::new()
                        .quote_style(if protect_bom {
                            csv::QuoteStyle::Always
                        } else {
                            csv::QuoteStyle::Necessary
                        })
                        .delimiter(options.delimiter)
                        .terminator(csv::Terminator::CRLF)
                        .from_writer(&mut output);
                    writer
                        .write_record(fields.iter().map(|v| {
                            let Value::String(s) = v else { unreachable!() };
                            s.as_str()
                        }))
                        .map_err(|e| resource(&format!("CSV encode failed: {e}")))?;
                    writer
                        .flush()
                        .map_err(|e| resource(&format!("CSV encode failed: {e}")))?;
                }
                Ok(Value::String(
                    String::from_utf8(output.data).expect("UTF-8 CSV fields"),
                ))
            },
        ),
    )?;
    let policy = Rc::new(policy.prepare()?);
    let owner = Rc::new(());
    let open_policy = policy.clone();
    let open_owner = owner.clone();
    runtime.register(
        "Csv::open",
        NativeFunction::new(
            NativeSignature::new(vec![Text], Object("CsvReader")).with_optional(optional),
            move |context, args| {
                let options = options(args.get(1))?;
                let path = paths::resolve(context, paths::string(&args[0])?)?;
                open_policy.check(context, &path, false)?;
                let input = BufReader::new(open_read(&path)?);
                Ok(Value::NativeObject(Rc::new(Reader {
                    state: RefCell::new(Some(Records {
                        input,
                        options,
                        width: None,
                        record: 0,
                    })),
                    owner: open_owner.clone(),
                })))
            },
        ),
    )?;
    runtime.register(
        "Csv::readRow",
        NativeFunction::new(
            NativeSignature::new(vec![Object("CsvReader")], Any),
            move |context, args| {
                let reader = reader(&args[0])?;
                if !Rc::ptr_eq(&reader.owner, &owner) {
                    return Err(NativeError::type_error(
                        "CsvReader belongs to another registration",
                    ));
                }
                context.permissions.require("filesystem.read")?;
                if !policy.read {
                    return Err(NativeError::new(
                        "PermissionError",
                        "filesystem policy denies reading",
                    ));
                }
                let mut state = reader.state.borrow_mut();
                let result = state
                    .as_mut()
                    .ok_or_else(|| NativeError::new("StreamError", "CsvReader is closed"))?
                    .next(context);
                if result.is_err() {
                    state.take();
                }
                result.map(|row| row.unwrap_or(Value::Null))
            },
        ),
    )?;
    runtime.register(
        "Csv::close",
        NativeFunction::new(
            NativeSignature::new(vec![Object("CsvReader")], Null),
            |_, args| {
                reader(&args[0])?.state.borrow_mut().take();
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(())
}
