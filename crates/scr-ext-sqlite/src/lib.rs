//! Optional built-in SQLite. File access, SQL work, and returned data are bounded.
use fallible_iterator::FallibleIterator;
use rusqlite::{
    Batch, Connection, ErrorCode, OpenFlags,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::{Value as SqlValue, ValueRef},
};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType,
    Runtime, RuntimeContext, Value,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_SQL_BYTES: usize = 1024 * 1024;
const MAX_CELL_BYTES: usize = 1024 * 1024;
const MAX_PARAMS: usize = 1024;
const PROGRESS_INTERVAL: usize = 1000;

/// Host limits apply to each native call. Roots are independent of stdlib policy.
#[derive(Debug, Clone)]
pub struct SqliteOptions {
    pub roots: Option<Vec<PathBuf>>,
    pub max_connections: usize,
    pub max_rows: usize,
    pub max_result_bytes: usize,
    pub max_steps: usize,
    pub statement_timeout: Duration,
}
impl Default for SqliteOptions {
    fn default() -> Self {
        Self {
            roots: None,
            max_connections: 64,
            max_rows: 10_000,
            max_result_bytes: 8 * 1024 * 1024,
            max_steps: 5_000_000,
            statement_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug)]
struct Database {
    connection: RefCell<Option<Connection>>,
    owner: Rc<()>,
    live: Rc<Cell<usize>>,
    file: bool,
    read_only: bool,
}
impl NativeObject for Database {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "SqliteDatabase"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.connection.borrow().is_none())),
            "readOnly" => Ok(Value::Bool(self.read_only)),
            _ => Err(format!("unknown SqliteDatabase property {name}")),
        }
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        if self.connection.get_mut().is_some() {
            self.live.set(self.live.get() - 1);
        }
    }
}
fn database<'a>(value: &'a Value, owner: &Rc<()>) -> NativeResult<&'a Database> {
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("expected SqliteDatabase"));
    };
    let db = object
        .as_any()
        .and_then(|v| v.downcast_ref::<Database>())
        .ok_or_else(|| NativeError::type_error("expected SqliteDatabase"))?;
    if !Rc::ptr_eq(&db.owner, owner) {
        return Err(NativeError::type_error(
            "SqliteDatabase belongs to another registration",
        ));
    }
    Ok(db)
}
fn error(kind: &str, message: impl Into<String>) -> NativeError {
    NativeError::new(kind, message)
}
fn limit(message: &str) -> NativeError {
    error("SqliteLimitError", message)
}
fn sql_error(e: rusqlite::Error, state: &AtomicU8) -> NativeError {
    match state.load(Ordering::Relaxed) {
        1 => return error("Interrupted", "SQLite execution interrupted"),
        2 => return limit("SQLite statement step limit exceeded"),
        3 => return limit("SQLite statement deadline exceeded"),
        4 => {
            return error(
                "PermissionError",
                "SQLite operation denied by connection policy",
            );
        }
        _ => {}
    }
    let kind = match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "SqliteBusyError",
        Some(ErrorCode::ConstraintViolation) => "SqliteConstraintError",
        Some(ErrorCode::TooBig) => "SqliteLimitError",
        Some(
            ErrorCode::ReadOnly
            | ErrorCode::PermissionDenied
            | ErrorCode::AuthorizationForStatementDenied,
        ) => "PermissionError",
        _ => "SqliteError",
    };
    error(kind, e.to_string())
}
fn plain_error(e: rusqlite::Error) -> NativeError {
    sql_error(e, &AtomicU8::new(0))
}

/// Register with unrestricted filesystem roots and the default resource limits.
pub fn register(runtime: &mut Runtime) -> NativeResult<()> {
    register_with_options(runtime, SqliteOptions::default())
}

/// Register once per runtime. Hosts restricting paths should supply SQLite roots.
pub fn register_with_options(
    runtime: &mut Runtime,
    mut options: SqliteOptions,
) -> NativeResult<()> {
    if options.max_connections == 0
        || options.max_rows == 0
        || options.max_result_bytes == 0
        || options.max_steps < PROGRESS_INTERVAL
        || options.statement_timeout.is_zero()
    {
        return Err(NativeError::type_error(
            "SQLite limits must be positive; max_steps must be at least 1000",
        ));
    }
    if let Some(roots) = &mut options.roots {
        for root in roots {
            if !root.is_absolute() {
                return Err(NativeError::type_error(
                    "SQLite roots must be absolute directories",
                ));
            }
            *root = std::fs::canonicalize(&*root)?;
            if !root.is_dir() {
                return Err(NativeError::type_error("SQLite root must be a directory"));
            }
        }
    }
    let options = Rc::new(options);
    let owner = Rc::new(());
    let live = Rc::new(Cell::new(0));
    let open_owner = owner.clone();
    let open_options = options.clone();
    runtime.register(
        "Sqlite::open",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::String],
                NativeType::Object("SqliteDatabase"),
            )
            .with_optional(vec![NativeType::Bool, NativeType::Int]),
            move |context, args| {
                context.check_cancelled()?;
                let Value::String(path) = &args[0] else {
                    unreachable!()
                };
                let read_only = matches!(args.get(1), Some(Value::Bool(true)));
                let timeout = match args.get(2) {
                    Some(Value::Int(n)) => *n,
                    _ => 1000,
                };
                if !(0..=5000).contains(&timeout) {
                    return Err(error(
                        "RangeError",
                        "busyTimeoutMs must be between 0 and 5000",
                    ));
                }
                if path.is_empty()
                    || path.contains('\0')
                    || path.starts_with("file:")
                    || path.contains("://")
                {
                    return Err(error(
                        "SqliteError",
                        "expected a local database path or :memory:; SQLite URIs are disabled",
                    ));
                }
                if live.get() >= open_options.max_connections {
                    return Err(limit("SQLite open connection limit exceeded"));
                }
                let file = path != ":memory:";
                let connection = if file {
                    context.permissions.require("filesystem.read")?;
                    if !read_only {
                        context.permissions.require("filesystem.write")?;
                    }
                    let path = resolve_path(context, path, open_options.roots.as_deref())?;
                    let flags = if read_only {
                        OpenFlags::SQLITE_OPEN_READ_ONLY
                    } else {
                        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
                    };
                    Connection::open_with_flags(
                        path,
                        flags | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_NOFOLLOW,
                    )
                    .map_err(plain_error)?
                } else {
                    Connection::open_in_memory().map_err(plain_error)?
                };
                connection
                    .busy_timeout(Duration::from_millis(timeout as u64))
                    .map_err(plain_error)?;
                connection
                    .pragma_update(None, "foreign_keys", true)
                    .map_err(plain_error)?;
                connection
                    .pragma_update(None, "temp_store", "MEMORY")
                    .map_err(plain_error)?;
                for (kind, value) in [
                    (Limit::SQLITE_LIMIT_LENGTH, MAX_CELL_BYTES as i32),
                    (Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_SQL_BYTES as i32),
                    (Limit::SQLITE_LIMIT_COLUMN, 256),
                    (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, MAX_PARAMS as i32),
                    (Limit::SQLITE_LIMIT_EXPR_DEPTH, 100),
                    (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 32),
                    (Limit::SQLITE_LIMIT_ATTACHED, 0),
                    (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
                ] {
                    connection.set_limit(kind, value).map_err(plain_error)?;
                }
                live.set(live.get() + 1);
                Ok(Value::NativeObject(Rc::new(Database {
                    connection: RefCell::new(Some(connection)),
                    owner: open_owner.clone(),
                    live: live.clone(),
                    file,
                    read_only,
                })))
            },
        ),
    )?;
    for query in [false, true] {
        let owner = owner.clone();
        let options = options.clone();
        runtime.register(
            if query {
                "Sqlite::query"
            } else {
                "Sqlite::execute"
            },
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::Object("SqliteDatabase"), NativeType::String],
                    if query {
                        NativeType::List
                    } else {
                        NativeType::Int
                    },
                )
                .with_optional(vec![NativeType::List]),
                move |context, args| {
                    let db = database(&args[0], &owner)?;
                    let Value::String(sql) = &args[1] else {
                        unreachable!()
                    };
                    let params = match args.get(2) {
                        Some(Value::List(values)) => values.as_slice(),
                        _ => &[],
                    };
                    operation(context, db, sql, params, query, &options)
                },
            ),
        )?;
    }
    runtime.register(
        "Sqlite::close",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("SqliteDatabase")], NativeType::Null),
            move |_, args| {
                let db = database(&args[0], &owner)?;
                let mut state = db.connection.borrow_mut();
                if let Some(connection) = state.take() {
                    if let Err((connection, e)) = connection.close() {
                        *state = Some(connection);
                        return Err(plain_error(e));
                    }
                    db.live.set(db.live.get() - 1);
                }
                Ok(Value::Null)
            },
        ),
    )?;
    Ok(())
}

fn resolve_path(
    context: &RuntimeContext<'_>,
    value: &str,
    roots: Option<&[PathBuf]>,
) -> NativeResult<PathBuf> {
    let path = context.working_directory.join(value);
    let resolved = match std::fs::canonicalize(&path) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| error("SqliteError", "database path needs a parent"))?;
            std::fs::canonicalize(parent)?.join(
                path.file_name()
                    .ok_or_else(|| error("SqliteError", "invalid database filename"))?,
            )
        }
        Err(e) => return Err(e.into()),
    };
    if roots.is_some_and(|roots| !roots.iter().any(|root| resolved.starts_with(root))) {
        return Err(error(
            "PermissionError",
            "database path is outside allowed SQLite roots",
        ));
    }
    Ok(resolved)
}

fn install_hooks(
    connection: &Connection,
    context: &RuntimeContext<'_>,
    db: &Database,
    options: &SqliteOptions,
) -> NativeResult<Arc<AtomicU8>> {
    let state = Arc::new(AtomicU8::new(0));
    let denied = state.clone();
    let writing =
        !db.read_only && (!db.file || context.permissions.allowed.contains("filesystem.write"));
    connection
        .authorizer(Some(move |auth: AuthContext<'_>| {
            use AuthAction::*;
            let allowed = match auth.action {
                // Files other than the checked main database must never be opened by SQL.
                Attach { .. }
                | Detach { .. }
                | Pragma { .. }
                | CreateVtable { .. }
                | DropVtable { .. }
                | Unknown { .. } => false,
                Function { function_name }
                    if function_name.eq_ignore_ascii_case("load_extension") =>
                {
                    false
                }
                Read { .. }
                | Select
                | Function { .. }
                | Transaction { .. }
                | Savepoint { .. }
                | Recursive => true,
                _ => writing,
            };
            if allowed {
                Authorization::Allow
            } else {
                denied.store(4, Ordering::Relaxed);
                Authorization::Deny
            }
        }))
        .map_err(plain_error)?;
    let cancellation = context.cancellation.clone();
    let progress = state.clone();
    let started = Instant::now();
    let timeout = options.statement_timeout;
    let mut remaining = options.max_steps;
    connection
        .progress_handler(
            PROGRESS_INTERVAL as i32,
            Some(move || {
                let reason = if cancellation
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed))
                {
                    1
                } else if started.elapsed() >= timeout {
                    3
                } else if remaining < PROGRESS_INTERVAL {
                    2
                } else {
                    remaining -= PROGRESS_INTERVAL;
                    0
                };
                if reason != 0 {
                    progress.store(reason, Ordering::Relaxed);
                }
                reason != 0
            }),
        )
        .map_err(plain_error)?;
    Ok(state)
}

fn operation(
    context: &RuntimeContext<'_>,
    db: &Database,
    sql: &str,
    params: &[Value],
    query: bool,
    options: &SqliteOptions,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    if db.file {
        context.permissions.require("filesystem.read")?;
    }
    let borrowed = db.connection.borrow();
    let connection = borrowed
        .as_ref()
        .ok_or_else(|| error("SqliteError", "SqliteDatabase is closed"))?;
    if sql.len() > MAX_SQL_BYTES {
        return Err(limit("SQLite SQL exceeds 1 MiB"));
    }
    if sql.contains('\0') {
        return Err(error("SqliteError", "SQL must not contain NUL"));
    }
    let budget = options
        .max_result_bytes
        .min(context.max_value_bytes.unwrap_or(usize::MAX));
    let parameters = parameters(params, budget)?;
    let state = install_hooks(connection, context, db, options)?;
    let result = (|| {
        // Batch prepares at most two statements, avoiding recursive preparation of
        // an arbitrarily long SQL tail. Neither statement is stepped here.
        let mut batch = Batch::new(connection, sql);
        let mut statement = batch
            .next()
            .map_err(|e| sql_error(e, &state))?
            .ok_or_else(|| error("SqliteError", "expected one SQL statement"))?;
        if batch.next().map_err(|e| sql_error(e, &state))?.is_some() {
            return Err(error("SqliteError", "expected exactly one SQL statement"));
        }
        if query && (!statement.readonly() || statement.column_count() == 0) {
            return Err(error(
                "SqliteError",
                "query requires a read-only statement returning columns",
            ));
        }
        if !query && statement.column_count() != 0 {
            return Err(error(
                "SqliteError",
                "execute does not accept statements returning columns; use query for SELECT",
            ));
        }
        for index in 1..=statement.parameter_count() {
            if statement
                .parameter_name(index)
                .is_some_and(|name| !name.starts_with('?'))
            {
                return Err(error("SqliteError", "use positional ? or ?NNN parameters"));
            }
        }
        if !query {
            let before = connection.total_changes();
            statement
                .execute(rusqlite::params_from_iter(&parameters))
                .map_err(|e| sql_error(e, &state))?;
            // SQLite's changes() retains the previous DML count for DDL/transactions.
            let count = if connection.total_changes() == before {
                0
            } else {
                connection.changes()
            };
            return Ok(Value::Int(
                i64::try_from(count).map_err(|_| limit("affected row count exceeds int range"))?,
            ));
        }
        let names: Vec<String> = statement
            .column_names()
            .iter()
            .map(|name| (*name).into())
            .collect();
        let mut unique = HashSet::new();
        if names.iter().any(|name| !unique.insert(name)) {
            return Err(error(
                "SqliteError",
                "duplicate result column names; use SQL aliases",
            ));
        }
        let mut used = 24usize;
        let mut rows = statement
            .query(rusqlite::params_from_iter(&parameters))
            .map_err(|e| sql_error(e, &state))?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(|e| sql_error(e, &state))? {
            context.check_cancelled()?;
            if result.len() >= options.max_rows {
                return Err(limit("SQLite query row limit exceeded"));
            }
            charge(&mut used, 32, budget)?;
            let mut values = Vec::with_capacity(names.len());
            for (index, name) in names.iter().enumerate() {
                charge(&mut used, 64 + name.len(), budget)?;
                let value = match row.get_ref(index).map_err(|e| sql_error(e, &state))? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::Int(n),
                    ValueRef::Real(n) if n.is_finite() => Value::Float(n),
                    ValueRef::Real(_) => {
                        return Err(error(
                            "SqliteTypeError",
                            "non-finite SQLite REAL is unsupported",
                        ));
                    }
                    ValueRef::Text(bytes) => {
                        charge(&mut used, bytes.len(), budget)?;
                        let text = std::str::from_utf8(bytes).map_err(|_| {
                            error("SqliteTypeError", "SQLite TEXT is not valid UTF-8")
                        })?;
                        Value::String(text.into())
                    }
                    ValueRef::Blob(_) => {
                        return Err(error(
                            "SqliteTypeError",
                            "SQLite BLOB results are unsupported; use SQL hex()",
                        ));
                    }
                };
                values.push((MapKey::String(name.clone()), value));
            }
            result.push(Value::Map(values));
        }
        charge(&mut used, 0, budget)?;
        Ok(Value::List(result))
    })();
    // Release cancellation flags and per-call counters after errors as well as success.
    let clear_progress = connection
        .progress_handler(0, None::<fn() -> bool>)
        .map_err(plain_error);
    let clear_authorizer = connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(plain_error);
    match result {
        Err(e) => Err(e),
        Ok(value) => {
            clear_progress?;
            clear_authorizer?;
            Ok(value)
        }
    }
}
fn charge(used: &mut usize, bytes: usize, budget: usize) -> NativeResult<()> {
    *used = used
        .checked_add(bytes)
        .ok_or_else(|| limit("SQLite result size overflow"))?;
    if *used > budget {
        return Err(limit("SQLite result byte limit exceeded"));
    }
    Ok(())
}
fn parameters(values: &[Value], budget: usize) -> NativeResult<Vec<SqlValue>> {
    if values.len() > MAX_PARAMS {
        return Err(limit("SQLite parameter count exceeds 1024"));
    }
    let mut used = 0;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        charge(&mut used, 24, budget)?;
        result.push(match value {
            Value::Null => SqlValue::Null,
            Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
            Value::Int(v) => SqlValue::Integer(*v),
            Value::Float(v) if v.is_finite() => SqlValue::Real(*v),
            Value::String(v) => {
                if v.len() > MAX_CELL_BYTES {
                    return Err(limit("SQLite parameter exceeds 1 MiB"));
                }
                charge(&mut used, v.len(), budget)?;
                SqlValue::Text(v.clone())
            }
            _ => {
                return Err(NativeError::type_error(
                    "SQLite parameters must be null, bool, int, finite float, or string",
                ));
            }
        });
    }
    Ok(result)
}
