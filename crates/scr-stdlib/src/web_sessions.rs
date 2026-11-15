//! Host-owned file sessions, shared across workers with stable locks and atomic records.
use super::{Response, cookies};
use fs2::FileExt;
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct Config {
    pub directory: PathBuf,
    pub ttl: Duration,
    pub max_sessions: usize,
    pub max_bytes: usize,
    pub secure: bool,
}
impl Config {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            ttl: Duration::from_secs(1800),
            max_sessions: 10_000,
            max_bytes: 64 * 1024,
            secure: true,
        }
    }
    /// The session directory is a dedicated host resource, independent of script file roots.
    /// Existing Unix directories must be private; permissions are never broadened.
    pub fn prepare(&mut self) -> NativeResult<()> {
        if !(1..=100_000).contains(&self.max_sessions)
            || !(1..=1024 * 1024).contains(&self.max_bytes)
            || self.ttl.as_secs() == 0
            || self.ttl > Duration::from_secs(2_592_000)
        {
            return Err(error("invalid session host limits"));
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.directory).map_err(storage)?;
        let metadata = fs::symlink_metadata(&self.directory).map_err(storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(error("session directory must be a real directory"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(error(
                    "session directory must have private Unix permissions (0700)",
                ));
            }
        }
        self.directory = fs::canonicalize(&self.directory).map_err(storage)?;
        Ok(())
    }
}
fn error(message: &str) -> NativeError {
    NativeError::new("SessionError", message)
}
fn storage(_: io::Error) -> NativeError {
    error("session storage operation failed")
}
fn now() -> NativeResult<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("system clock precedes Unix epoch"))?
        .as_secs())
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn new_id() -> NativeResult<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| error("session random ID generation failed"))?;
    const HEX: &[u8] = b"0123456789abcdef";
    let mut id = String::with_capacity(64);
    for byte in bytes {
        id.push(HEX[(byte >> 4) as usize] as char);
        id.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(id)
}
fn exists(path: &Path) -> NativeResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(storage(e)),
    }
}
fn check_file(path: &Path) -> NativeResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(storage)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(error("session storage entries must be regular files"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 || metadata.permissions().mode() & 0o077 != 0 {
            return Err(error(
                "session storage files must be private and unlinked elsewhere",
            ));
        }
    }
    Ok(())
}
fn open(path: &Path, create: bool) -> NativeResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if create {
        match options.create_new(true).open(path) {
            Ok(file) => return Ok(file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(storage(e)),
        }
    }
    check_file(path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(storage)
}
fn contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}
fn lock(file: &File, check: &impl Fn() -> NativeResult<()>) -> NativeResult<()> {
    let started = Instant::now();
    loop {
        check()?;
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(()),
            Err(e) if contended(&e) => {
                if started.elapsed() > Duration::from_secs(30) {
                    return Err(error("session lock wait exceeded 30 seconds"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(storage(e)),
        }
    }
}
fn store_lock(config: &Config, check: &impl Fn() -> NativeResult<()>) -> NativeResult<File> {
    let file = open(&config.directory.join(".store.lock"), true)?;
    lock(&file, check)?;
    Ok(file)
}
fn paths(config: &Config, id: &str) -> (PathBuf, PathBuf) {
    (
        config.directory.join(format!("{id}.lock")),
        config.directory.join(format!("{id}.data")),
    )
}
fn expiry(path: &Path) -> NativeResult<u64> {
    check_file(path)?;
    let mut bytes = [0; 8];
    File::open(path)
        .map_err(storage)?
        .read_exact(&mut bytes)
        .map_err(storage)?;
    Ok(u64::from_be_bytes(bytes))
}
fn atomic_record(config: &Config, id: &str, expires: u64, bytes: &[u8]) -> NativeResult<()> {
    if bytes.len() > config.max_bytes {
        return Err(error("session value byte limit exceeded"));
    }
    let (_, path) = paths(config, id);
    if exists(&path)? {
        check_file(&path)?;
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".session-tmp-")
        .tempfile_in(&config.directory)
        .map_err(storage)?;
    temporary
        .write_all(&expires.to_be_bytes())
        .map_err(storage)?;
    temporary.write_all(bytes).map_err(storage)?;
    temporary.as_file().sync_all().map_err(storage)?;
    temporary.persist(path).map_err(|e| storage(e.error))?;
    Ok(())
}
// Caller holds the store lock. Per-record lock attempts are nonblocking, so a
// long-running request never blocks unrelated sessions or deadlocks regeneration.
fn collect(config: &Config, check: &impl Fn() -> NativeResult<()>) -> NativeResult<usize> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(&config.directory).map_err(storage)? {
        check()?;
        if entries.len() > config.max_sessions * 2 + 256 {
            return Err(error("session directory entry limit exceeded"));
        }
        entries.push(entry.map_err(storage)?);
    }
    let time = now()?;
    let mut active = 0;
    for entry in entries {
        check()?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| error("unexpected session directory entry"))?;
        if name == ".store.lock" {
            continue;
        }
        if name.starts_with(".session-tmp-") {
            check_file(&entry.path())?;
            fs::remove_file(entry.path()).map_err(storage)?;
            continue;
        }
        if let Some(id) = name.strip_suffix(".data").filter(|id| valid_id(id)) {
            let (lock_path, _) = paths(config, id);
            if !exists(&lock_path)? && exists(&entry.path())? {
                check_file(&entry.path())?;
                fs::remove_file(entry.path()).map_err(storage)?;
            }
            continue;
        }
        let id = name
            .strip_suffix(".lock")
            .filter(|id| valid_id(id))
            .ok_or_else(|| error("unexpected session directory entry"))?;
        let (lock_path, data_path) = paths(config, id);
        let file = open(&lock_path, false)?;
        match file.try_lock_exclusive() {
            Err(e) if contended(&e) => {
                active += 1;
                continue;
            }
            Err(e) => return Err(storage(e)),
            Ok(()) => {}
        }
        let expired = if exists(&data_path)? {
            expiry(&data_path)? <= time
        } else {
            true
        };
        if expired {
            if exists(&data_path)? {
                fs::remove_file(data_path).map_err(storage)?;
            }
            drop(file);
            fs::remove_file(lock_path).map_err(storage)?;
        } else {
            active += 1;
        }
    }
    Ok(active)
}
struct Lease {
    id: String,
    _file: File,
}
fn create(
    config: &Config,
    credit: usize,
    check: &impl Fn() -> NativeResult<()>,
) -> NativeResult<Lease> {
    let _store = store_lock(config, check)?;
    if collect(config, check)? >= config.max_sessions + credit {
        return Err(error("session store capacity reached"));
    }
    for _ in 0..8 {
        check()?;
        let id = new_id()?;
        let (path, _) = paths(config, &id);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = match options.open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(storage(e)),
        };
        lock(&file, check)?;
        atomic_record(config, &id, 0, br#"{"Map":[]}"#)?;
        return Ok(Lease { id, _file: file });
    }
    Err(error("could not allocate a unique session ID"))
}
fn resume(
    config: &Config,
    id: &str,
    check: &impl Fn() -> NativeResult<()>,
) -> NativeResult<Option<(Lease, Vec<u8>)>> {
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(30) {
            return Err(error("session lock wait exceeded 30 seconds"));
        }
        check()?;
        let store = store_lock(config, check)?;
        let (path, data) = paths(config, id);
        if !exists(&path)? {
            return Ok(None);
        }
        let file = open(&path, false)?;
        match file.try_lock_exclusive() {
            Err(e) if contended(&e) => {
                drop(store);
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(e) => return Err(storage(e)),
            Ok(()) => {}
        }
        drop(store);
        if !exists(&data)? || expiry(&data)? <= now()? {
            return Ok(None);
        }
        check_file(&data)?;
        let mut bytes = Vec::new();
        File::open(data)
            .map_err(storage)?
            .take((config.max_bytes as u64) + 9)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() < 8 || bytes.len() > config.max_bytes + 8 {
            return Err(error("session record exceeds storage bounds"));
        }
        bytes.drain(..8);
        return Ok(Some((
            Lease {
                id: id.into(),
                _file: file,
            },
            bytes,
        )));
    }
}
#[derive(Serialize, Deserialize)]
enum Key {
    Int(i64),
    String(String),
}
#[derive(Serialize, Deserialize)]
enum Stored {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Stored>),
    Map(Vec<(Key, Stored)>),
}
fn measure(
    value: &Value,
    depth: usize,
    total: &mut usize,
    limit: usize,
    check: &impl Fn() -> NativeResult<()>,
) -> NativeResult<()> {
    check()?;
    if depth > 32 {
        return Err(error("session values exceed 32 nesting levels"));
    }
    *total = total.saturating_add(std::mem::size_of::<Value>());
    match value {
        Value::String(s) => *total = total.saturating_add(s.len()),
        Value::Bytes(b) => *total = total.saturating_add(b.len()),
        Value::List(values) => {
            for value in values {
                measure(value, depth + 1, total, limit, check)?;
            }
        }
        Value::Map(entries) => {
            for (key, value) in entries {
                *total = total.saturating_add(std::mem::size_of::<MapKey>());
                if let MapKey::String(s) = key {
                    *total = total.saturating_add(s.len());
                }
                measure(value, depth + 1, total, limit, check)?;
            }
        }
        Value::Float(f) if !f.is_finite() => return Err(error("session floats must be finite")),
        Value::NativeObject(_) | Value::Callable(_) => {
            return Err(error("sessions cannot store native resources or callables"));
        }
        _ => {}
    }
    if *total > limit {
        return Err(error("session value byte limit exceeded"));
    }
    Ok(())
}
fn stored(value: &Value) -> Stored {
    match value {
        Value::Null => Stored::Null,
        Value::Bool(v) => Stored::Bool(*v),
        Value::Int(v) => Stored::Int(*v),
        Value::Float(v) => Stored::Float(*v),
        Value::String(v) => Stored::String(v.clone()),
        Value::Bytes(v) => Stored::Bytes(v.to_vec()),
        Value::List(v) => Stored::List(v.iter().map(stored).collect()),
        Value::Map(v) => Stored::Map(
            v.iter()
                .map(|(k, v)| {
                    (
                        match k {
                            MapKey::Int(i) => Key::Int(*i),
                            MapKey::String(s) => Key::String(s.clone()),
                        },
                        stored(v),
                    )
                })
                .collect(),
        ),
        _ => unreachable!(),
    }
}
fn value(stored: Stored) -> Value {
    match stored {
        Stored::Null => Value::Null,
        Stored::Bool(v) => Value::Bool(v),
        Stored::Int(v) => Value::Int(v),
        Stored::Float(v) => Value::Float(v),
        Stored::String(v) => Value::String(v),
        Stored::Bytes(v) => Value::Bytes(v.into()),
        Stored::List(v) => Value::List(v.into_iter().map(value).collect()),
        Stored::Map(v) => Value::Map(
            v.into_iter()
                .map(|(k, v)| {
                    (
                        match k {
                            Key::Int(i) => MapKey::Int(i),
                            Key::String(s) => MapKey::String(s),
                        },
                        value(v),
                    )
                })
                .collect(),
        ),
    }
}
struct Buffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("session encoded byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(data: &Value, config: &Config, context: &RuntimeContext<'_>) -> NativeResult<Vec<u8>> {
    measure(data, 0, &mut 0, config.max_bytes, &|| {
        context.check_cancelled()
    })?;
    let mut output = Buffer {
        bytes: vec![],
        limit: config.max_bytes,
    };
    serde_json::to_writer(&mut output, &stored(data))
        .map_err(|_| error("session encoded byte limit exceeded"))?;
    Ok(output.bytes)
}
struct State {
    config: Config,
    cookie: Option<String>,
    leases: Vec<Lease>,
    data: Value,
    encoded: Vec<u8>,
    started: bool,
    destroyed: bool,
    closed: bool,
    response: Rc<RefCell<Response>>,
    cancellation: Option<Arc<AtomicBool>>,
}
impl State {
    fn active(&self) -> NativeResult<()> {
        if !self.started || self.destroyed || self.closed {
            Err(error("session must be started, open, and not destroyed"))
        } else {
            Ok(())
        }
    }
    fn start(&mut self, context: &RuntimeContext<'_>) -> NativeResult<()> {
        if self.destroyed || self.closed {
            return Err(error(
                "a destroyed session cannot restart in the same request",
            ));
        }
        if self.started {
            return Ok(());
        }
        let resumed = if let Some(id) = &self.cookie {
            resume(&self.config, id, &|| context.check_cancelled())?
        } else {
            None
        };
        let (lease, encoded) = if let Some(record) = resumed {
            record
        } else {
            (
                create(&self.config, 0, &|| context.check_cancelled())?,
                br#"{"Map":[]}"#.to_vec(),
            )
        };
        let data =
            value(serde_json::from_slice(&encoded).map_err(|_| error("invalid session record"))?);
        if !matches!(data, Value::Map(_)) {
            return Err(error("session record must contain a map"));
        }
        measure(&data, 0, &mut 0, self.config.max_bytes, &|| {
            context.check_cancelled()
        })?;
        cookies::managed_session_cookie(
            &mut self.response.borrow_mut(),
            &lease.id,
            self.config.secure,
            self.config.ttl.as_secs(),
            false,
        )?;
        self.data = data;
        self.encoded = encoded;
        self.leases.push(lease);
        self.started = true;
        self.cancellation = context.cancellation.clone();
        Ok(())
    }
}
/// Keeps the record lease alive until the request finishes. Dropping without
/// finish discards mutations; the OS releases file locks even after worker death.
#[derive(Clone)]
pub struct Session(Rc<RefCell<State>>);
impl Session {
    pub fn finish(&self) -> NativeResult<()> {
        let mut state = self.0.borrow_mut();
        if !state.started || state.closed {
            return Ok(());
        }
        let check = || {
            if state
                .cancellation
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                Err(error("session commit interrupted"))
            } else {
                Ok(())
            }
        };
        let _store = store_lock(&state.config, &check)?;
        // Validate final cookie/header space before changing persistent state.
        let current = state.leases.last().unwrap();
        let mut response = state.response.borrow().clone();
        cookies::managed_session_cookie(
            &mut response,
            &current.id,
            state.config.secure,
            state.config.ttl.as_secs(),
            state.destroyed,
        )?;
        let expires = if state.destroyed {
            0
        } else {
            now()?.saturating_add(state.config.ttl.as_secs())
        };
        atomic_record(
            &state.config,
            &current.id,
            expires,
            if state.destroyed {
                br#"{"Map":[]}"#
            } else {
                &state.encoded
            },
        )?;
        for old in state.leases.iter().take(state.leases.len() - 1) {
            atomic_record(&state.config, &old.id, 0, br#"{"Map":[]}"#)?;
        }
        *state.response.borrow_mut() = response;
        state.closed = true;
        state.leases.clear();
        Ok(())
    }
}
/// Register request-local Session APIs with access only to the host's dedicated store.
pub fn register(
    runtime: &mut Runtime,
    config: Config,
    headers: &[(String, String)],
    response: Rc<RefCell<Response>>,
) -> NativeResult<Session> {
    // Invalid/ambiguous session cookies are rejected only when start() is used;
    // ordinary endpoints can still handle malformed cookies themselves.
    let cookies = cookies::request_cookies(headers);
    let (cookie, invalid) = match cookies {
        Ok(pairs) => {
            let ids: Vec<_> = pairs
                .into_iter()
                .filter(|(name, _)| name == "SCRIPTA_SESSION")
                .collect();
            if ids.len() > 1 {
                (None, true)
            } else {
                (ids.into_iter().next().map(|(_, value)| value), false)
            }
        }
        Err(_) => (None, true),
    };
    let state = Rc::new(RefCell::new(State {
        config,
        cookie: cookie.filter(|id| valid_id(id)),
        leases: vec![],
        data: Value::Map(vec![]),
        encoded: vec![],
        started: false,
        destroyed: false,
        closed: false,
        response,
        cancellation: None,
    }));
    for name in [
        "start",
        "id",
        "get",
        "has",
        "set",
        "remove",
        "all",
        "regenerate",
        "destroy",
        "close",
    ] {
        let state = state.clone();
        let (parameters, returns) = match name {
            "get" => (vec![NativeType::String], NativeType::Any),
            "has" | "remove" => (vec![NativeType::String], NativeType::Bool),
            "set" => (vec![NativeType::String, NativeType::Any], NativeType::Null),
            "id" => (vec![], NativeType::String),
            "all" => (vec![], NativeType::Map),
            _ => (vec![], NativeType::Null),
        };
        let signature = if name == "get" {
            NativeSignature::new(parameters, returns).with_optional(vec![NativeType::Any])
        } else {
            NativeSignature::new(parameters, returns)
        };
        runtime.register(
            format!("Session::{name}"),
            NativeFunction::new(signature, move |context, args| {
                context.check_cancelled()?;
                let state_ref = state.clone();
                let mut state = state.borrow_mut();
                if name == "start" {
                    if invalid {
                        return Err(error("malformed or duplicate session cookie"));
                    }
                    state.start(context)?;
                    return Ok(Value::Null);
                }
                state.active()?;
                if name == "close" {
                    drop(state);
                    Session(state_ref).finish()?;
                    return Ok(Value::Null);
                }
                match name {
                    "id" => Ok(Value::String(state.leases.last().unwrap().id.clone())),
                    "all" => Ok(state.data.clone()),
                    "get" | "has" | "remove" | "set" => {
                        let Value::String(key) = &args[0] else {
                            unreachable!()
                        };
                        let Value::Map(entries) = &state.data else {
                            unreachable!()
                        };
                        let index = entries
                            .iter()
                            .position(|(k, _)| k == &MapKey::String(key.clone()));
                        if name == "get" {
                            return Ok(index
                                .map(|i| entries[i].1.clone())
                                .unwrap_or_else(|| args.get(1).cloned().unwrap_or(Value::Null)));
                        }
                        if name == "has" {
                            return Ok(Value::Bool(index.is_some()));
                        }
                        if name == "set" {
                            measure(&args[1], 0, &mut 0, state.config.max_bytes, &|| {
                                context.check_cancelled()
                            })?;
                        }
                        let mut candidate = entries.clone();
                        if name == "remove" {
                            if let Some(index) = index {
                                candidate.remove(index);
                            }
                        } else if let Some(index) = index {
                            candidate[index].1 = args[1].clone();
                        } else {
                            candidate.push((MapKey::String(key.clone()), args[1].clone()));
                        }
                        let candidate = Value::Map(candidate);
                        let encoded = encode(&candidate, &state.config, context)?;
                        state.data = candidate;
                        state.encoded = encoded;
                        Ok(if name == "remove" {
                            Value::Bool(index.is_some())
                        } else {
                            Value::Null
                        })
                    }
                    "regenerate" => {
                        if state.leases.len() > 1 {
                            return Err(error("session ID can regenerate only once per request"));
                        }
                        let lease = create(&state.config, 1, &|| context.check_cancelled())?;
                        cookies::managed_session_cookie(
                            &mut state.response.borrow_mut(),
                            &lease.id,
                            state.config.secure,
                            state.config.ttl.as_secs(),
                            false,
                        )?;
                        state.leases.push(lease);
                        Ok(Value::Null)
                    }
                    "destroy" => {
                        cookies::managed_session_cookie(
                            &mut state.response.borrow_mut(),
                            "",
                            state.config.secure,
                            state.config.ttl.as_secs(),
                            true,
                        )?;
                        state.destroyed = true;
                        Ok(Value::Null)
                    }
                    _ => unreachable!(),
                }
            }),
        )?;
    }
    Ok(Session(state))
}
