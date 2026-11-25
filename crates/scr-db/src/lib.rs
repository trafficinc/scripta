//! Shared database registration and runtime-owned handles.
//!
//! Backends implement [`Driver`] and [`Connection`]. Registration never connects
//! to a server. The shared interface supports typed connection, prepared execution,
//! typed result fetching, execution metadata, transactions, and explicit cleanup.
mod error_info;
pub use error_info::{ErrorCategory, ErrorDetails};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fmt,
    rc::{Rc, Weak},
};

/// Connector API metadata, independent of the database server/client version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverInfo {
    pub name: &'static str,
    pub version: &'static str,
    /// Supported connector operations, not server-specific feature negotiation.
    pub capabilities: &'static [&'static str],
}

/// Borrowed credentials and options. Intentionally does not implement Debug.
pub struct ConnectRequest<'a> {
    pub dsn: &'a str,
    pub user: &'a str,
    pub password: &'a str,
    pub options: &'a Value,
}

/// Transaction control or server-state inspection, without raw SQL from scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionOperation {
    Begin,
    Commit,
    RollBack,
    Inspect,
}

/// Trusted host backend; implementations enforce their own TLS and I/O policies.
pub trait Driver: fmt::Debug {
    fn info(&self) -> DriverInfo;
    fn connect(
        &self,
        context: &mut RuntimeContext<'_>,
        request: ConnectRequest<'_>,
    ) -> NativeResult<Box<dyn Connection>>;
}

/// One backend session. Its Drop must release resources without script cleanup.
///
/// Statement identifiers are unique among live statements on this connection.
/// The backend owns statements/cursors, avoiding a borrow of a connection inside
/// a script handle. Explicit close must release resources even when it fails.
pub trait Connection: fmt::Debug {
    /// Take details of the immediately preceding operation; never return stale
    /// details for a later preflight error. NativeError messages must be sanitized.
    fn take_error_details(&mut self) -> Option<ErrorDetails> {
        None
    }
    /// Broken I/O must invalidate every dependent script handle.
    fn is_broken(&self) -> bool {
        false
    }
    /// Retained logical payload beyond the outer native handle slot.
    fn payload_bytes(&self) -> usize {
        0
    }
    fn get_attribute(&self, _name: &str) -> NativeResult {
        Err(NativeError::new(
            "DbUnsupportedError",
            "database attribute is not supported",
        ))
    }
    fn set_attribute(&mut self, _name: &str, _value: &Value) -> NativeResult<()> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "database attribute is not supported",
        ))
    }
    fn prepare(&mut self, context: &mut RuntimeContext<'_>, sql: &str) -> NativeResult<u64>;
    fn execute(
        &mut self,
        _context: &mut RuntimeContext<'_>,
        _id: u64,
        _params: &Value,
    ) -> NativeResult<()> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "statement execution is not supported",
        ))
    }
    fn close_cursor(&mut self, _context: &mut RuntimeContext<'_>, _id: u64) -> NativeResult<()> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "cursor closure is not supported",
        ))
    }
    fn fetch(&mut self, _context: &mut RuntimeContext<'_>, _id: u64) -> NativeResult {
        Err(NativeError::new(
            "DbUnsupportedError",
            "row fetching is not supported",
        ))
    }
    fn fetch_all(&mut self, _context: &mut RuntimeContext<'_>, _id: u64) -> NativeResult {
        Err(NativeError::new(
            "DbUnsupportedError",
            "row fetching is not supported",
        ))
    }
    fn fetch_column(
        &mut self,
        _context: &mut RuntimeContext<'_>,
        _id: u64,
        _column: i64,
    ) -> NativeResult {
        Err(NativeError::new(
            "DbUnsupportedError",
            "column fetching is not supported",
        ))
    }
    fn exec(&mut self, _context: &mut RuntimeContext<'_>, _sql: &str) -> NativeResult<i64> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "exec is not supported",
        ))
    }
    fn row_count(&self, _id: u64) -> NativeResult<i64> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "row counts are not supported",
        ))
    }
    /// Backends with sequence lookups must use the supplied context for bounded
    /// network I/O and permissions; local metadata implementations may ignore it.
    fn last_insert_id(
        &mut self,
        _context: &mut RuntimeContext<'_>,
        _sequence: Option<&str>,
    ) -> NativeResult {
        Err(NativeError::new(
            "DbUnsupportedError",
            "insert IDs are not supported",
        ))
    }
    fn transaction(
        &mut self,
        _context: &mut RuntimeContext<'_>,
        _operation: TransactionOperation,
    ) -> NativeResult<bool> {
        Err(NativeError::new(
            "DbUnsupportedError",
            "transactions are not supported",
        ))
    }
    /// Pure preflight; rejection must leave the statement and connection intact.
    fn validate_close_statement(&self, _id: u64) -> NativeResult<()> {
        Ok(())
    }
    fn close_statement(&mut self, context: &mut RuntimeContext<'_>, id: u64) -> NativeResult<()>;
    fn close(&mut self, context: &mut RuntimeContext<'_>) -> NativeResult<()>;
}

#[derive(Debug, Clone)]
pub struct DatabaseOptions {
    pub max_connections: usize,
    pub max_statements_per_connection: usize,
    pub max_sql_bytes: usize,
}
impl Default for DatabaseOptions {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_statements_per_connection: 256,
            max_sql_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug)]
struct Registration {
    owner: Rc<()>,
    drivers: BTreeMap<&'static str, Rc<dyn Driver>>,
    options: DatabaseOptions,
    live: Cell<usize>,
}

struct Session {
    myself: Weak<Session>,
    registration: Rc<Registration>,
    backend: RefCell<Option<Box<dyn Connection>>>,
    statements: RefCell<BTreeMap<u64, Weak<Statement>>>,
    error: RefCell<Option<Rc<error_info::Record>>>,
}
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbConnection")
            .field("closed", &self.backend.borrow().is_none())
            .finish_non_exhaustive()
    }
}
impl Session {
    fn observe<T>(
        &self,
        operation: &'static str,
        statement: Option<&Statement>,
        clear: impl FnOnce(&T) -> bool,
        result: &NativeResult<T>,
        details: Option<ErrorDetails>,
    ) {
        match result {
            Ok(value) if clear(value) => {
                self.error.borrow_mut().take();
                if let Some(statement) = statement {
                    statement.error.borrow_mut().take();
                }
            }
            Err(error) if !error_info::ignored(error) => {
                let record = Rc::new(error_info::Record::new(operation, error, details));
                *self.error.borrow_mut() = Some(record.clone());
                if let Some(statement) = statement {
                    *statement.error.borrow_mut() = Some(record);
                }
            }
            _ => {}
        }
    }
    fn failure(
        &self,
        operation: &'static str,
        statement: Option<&Statement>,
        error: NativeError,
    ) -> NativeError {
        self.observe::<()>(operation, statement, |_| false, &Err(error.clone()), None);
        error
    }
    fn call<T>(
        &self,
        operation: &'static str,
        statement: Option<&Statement>,
        clear: impl FnOnce(&T) -> bool,
        action: impl FnOnce(&mut dyn Connection) -> NativeResult<T>,
    ) -> NativeResult<T> {
        let (result, broken, details) = {
            let mut backend = self.backend.borrow_mut();
            match backend.as_mut() {
                Some(backend) => {
                    backend.take_error_details();
                    let result = action(backend.as_mut());
                    let details = backend.take_error_details();
                    (result, backend.is_broken(), details)
                }
                None => (Err(state("database connection is closed")), false, None),
            }
        };
        self.observe(operation, statement, clear, &result, details);
        if broken {
            drop(self.release());
        }
        result
    }
    fn shared(&self) -> NativeResult<Rc<Self>> {
        self.myself
            .upgrade()
            .ok_or_else(|| state("database connection owner is unavailable"))
    }
    fn release(&self) -> Option<Box<dyn Connection>> {
        let backend = self.backend.borrow_mut().take();
        if backend.is_some() {
            self.registration.live.set(self.registration.live.get() - 1);
            for statement in self.statements.borrow_mut().values() {
                if let Some(statement) = statement.upgrade() {
                    statement.closed.set(true);
                }
            }
            self.statements.borrow_mut().clear();
        }
        backend
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        // Backend Drop owns non-script cleanup, including open transactions.
        drop(self.release());
    }
}
impl NativeObject for Session {
    fn payload_bytes(&self) -> usize {
        self.backend
            .borrow()
            .as_ref()
            .map_or(0, |backend| backend.payload_bytes())
            .saturating_add(
                self.error
                    .borrow()
                    .as_ref()
                    .map_or(0, |error| error.payload()),
            )
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "DbConnection"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.backend.borrow().is_none())),
            _ => Err(format!("unknown DbConnection property {name}")),
        }
    }
}

struct Statement {
    session: Rc<Session>,
    id: u64,
    closed: Cell<bool>,
    error: RefCell<Option<Rc<error_info::Record>>>,
}
impl fmt::Debug for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbStatement")
            .field("closed", &self.closed.get())
            .finish_non_exhaustive()
    }
}
impl NativeObject for Statement {
    fn payload_bytes(&self) -> usize {
        let error = self.error.borrow();
        let connection_error = self.session.error.borrow();
        let bytes = error.as_ref().map_or(0, |error| {
            if connection_error
                .as_ref()
                .is_some_and(|other| Rc::ptr_eq(error, other))
            {
                0
            } else {
                error.payload()
            }
        });
        self.session.payload_bytes().saturating_add(bytes)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        "DbStatement"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "closed" => Ok(Value::Bool(self.closed.get())),
            _ => Err(format!("unknown DbStatement property {name}")),
        }
    }
}
// No RuntimeContext is available when a statement handle is dropped. Unclosed
// backend statements remain session-owned and count against the limit until
// teardown. Closed handles must not remove an ID again: a backend may have
// already reused that ID for a newer statement.

fn state(message: &str) -> NativeError {
    NativeError::new("DbStateError", message)
}
fn limit(message: &str) -> NativeError {
    NativeError::new("DbLimitError", message)
}
fn require_owner(context: &RuntimeContext<'_>, registration: &Registration) -> NativeResult<()> {
    if context
        .runtime_identity
        .as_ref()
        .is_some_and(|owner| Rc::ptr_eq(owner, &registration.owner))
    {
        Ok(())
    } else {
        Err(NativeError::type_error(
            "database handle belongs to another runtime",
        ))
    }
}
fn session<'a>(value: &'a Value, registration: &Registration) -> NativeResult<&'a Session> {
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("expected DbConnection"));
    };
    let session = object
        .as_any()
        .and_then(|object| object.downcast_ref::<Session>())
        .ok_or_else(|| NativeError::type_error("expected DbConnection"))?;
    if !Rc::ptr_eq(&session.registration.owner, &registration.owner)
        || !std::ptr::eq(session.registration.as_ref(), registration)
    {
        return Err(NativeError::type_error(
            "database handle belongs to another registration",
        ));
    }
    Ok(session)
}
fn statement<'a>(value: &'a Value, registration: &Registration) -> NativeResult<&'a Statement> {
    let Value::NativeObject(object) = value else {
        return Err(NativeError::type_error("expected DbStatement"));
    };
    let statement = object
        .as_any()
        .and_then(|object| object.downcast_ref::<Statement>())
        .ok_or_else(|| NativeError::type_error("expected DbStatement"))?;
    if !std::ptr::eq(statement.session.registration.as_ref(), registration) {
        return Err(NativeError::type_error(
            "database handle belongs to another registration",
        ));
    }
    Ok(statement)
}

/// Register the shared namespace once, supplying all enabled drivers together.
/// On any registration error, discard the partially initialized runtime.
pub fn register(runtime: &mut Runtime, drivers: Vec<Rc<dyn Driver>>) -> NativeResult<()> {
    register_with_options(runtime, drivers, DatabaseOptions::default())
}

pub fn register_with_options(
    runtime: &mut Runtime,
    drivers: Vec<Rc<dyn Driver>>,
    options: DatabaseOptions,
) -> NativeResult<()> {
    if options.max_connections == 0
        || options.max_statements_per_connection == 0
        || options.max_sql_bytes == 0
    {
        return Err(NativeError::type_error("database limits must be positive"));
    }
    let mut names = BTreeSet::new();
    for driver in &drivers {
        let info = driver.info();
        if info.name.is_empty()
            || !info.name.bytes().all(|byte| byte.is_ascii_lowercase())
            || info.version.is_empty()
            || !names.insert(info.name)
        {
            return Err(NativeError::new(
                "RegistryError",
                "invalid or duplicate database driver metadata",
            ));
        }
    }
    let registration = Rc::new(Registration {
        owner: runtime.native_identity(),
        drivers: drivers
            .into_iter()
            .map(|driver| (driver.info().name, driver))
            .collect(),
        options,
        live: Cell::new(0),
    });
    let registered = registration.clone();
    runtime.register(
        "Db::connect",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::String, NativeType::String, NativeType::String],
                NativeType::Object("DbConnection"),
            )
            .with_optional(vec![NativeType::OneOf(vec![
                NativeType::Map,
                NativeType::List,
            ])]),
            move |context, args| {
                require_owner(context, &registered)?;
                context.check_cancelled()?;
                context.permissions.require("network.connect")?;
                let [
                    Value::String(dsn),
                    Value::String(user),
                    Value::String(password),
                    ..,
                ] = args
                else {
                    return Err(NativeError::type_error(
                        "Db::connect requires DSN, user, and password strings",
                    ));
                };
                let default_options = Value::List(Vec::new());
                let options = args.get(3).unwrap_or(&default_options);
                match options {
                    Value::List(values) if values.is_empty() => {}
                    Value::Map(values)
                        if values
                            .iter()
                            .all(|(key, _)| matches!(key, scr_runtime::MapKey::String(_))) => {}
                    _ => {
                        return Err(NativeError::type_error(
                            "database options must be a string-keyed map or empty list",
                        ));
                    }
                }
                let (name, _) = dsn
                    .split_once(':')
                    .ok_or_else(|| NativeError::new("DbError", "invalid database DSN"))?;
                let driver = registered.drivers.get(name).ok_or_else(|| {
                    NativeError::new(
                        "DbUnsupportedError",
                        if name == "mysql" {
                            "MySQL driver is unavailable in this executable or host registration; inspect scripta version --verbose; CLI source builds need --features mysql, embedding hosts must register the driver; installing a script package cannot enable it"
                        } else if name == "pgsql" {
                            "PostgreSQL driver is unavailable in this executable or host registration; inspect scripta version --verbose; CLI source builds need --features postgres, embedding hosts must register the driver; installing a script package cannot enable it"
                        } else {
                            "database driver is unavailable in this executable or host registration; inspect scripta version --verbose and use a compatible executable or register the required driver"
                        },
                    )
                })?;
                if registered.live.get() >= registered.options.max_connections {
                    return Err(limit("database connection limit exceeded"));
                }
                let backend = driver.connect(
                    context,
                    ConnectRequest {
                        dsn,
                        user,
                        password,
                        options,
                    },
                )?;
                context.check_cancelled()?;
                registered.live.set(registered.live.get() + 1);
                Ok(Value::NativeObject(Rc::new_cyclic(|myself| Session {
                    myself: myself.clone(),
                    registration: registered.clone(),
                    backend: RefCell::new(Some(backend)),
                    statements: RefCell::new(BTreeMap::new()),
                    error: RefCell::new(None),
                })))
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::getAttribute",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Object("DbConnection"), NativeType::String],
                NativeType::Any,
            ),
            move |context, args| {
                require_owner(context, &registered)?;
                context.check_cancelled()?;
                let connection = session(&args[0], &registered)?;
                let Value::String(name) = &args[1] else {
                    return Err(NativeError::type_error("attribute name must be a string"));
                };
                connection.call(
                    "getAttribute",
                    None,
                    |_| false,
                    |backend| backend.get_attribute(name),
                )
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::setAttribute",
        NativeFunction::new(
            NativeSignature::new(
                vec![
                    NativeType::Object("DbConnection"),
                    NativeType::String,
                    NativeType::Any,
                ],
                NativeType::Null,
            ),
            move |context, args| {
                require_owner(context, &registered)?;
                context.check_cancelled()?;
                let connection = session(&args[0], &registered)?;
                let Value::String(name) = &args[1] else {
                    return Err(NativeError::type_error("attribute name must be a string"));
                };
                connection.call(
                    "setAttribute",
                    None,
                    |_| true,
                    |backend| backend.set_attribute(name, &args[2]),
                )?;
                Ok(Value::Null)
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::exec",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Object("DbConnection"), NativeType::String],
                NativeType::Int,
            ),
            move |context, args| {
                require_owner(context, &registered)?;
                let connection = session(&args[0], &registered)?;
                context.check_cancelled()?;
                context.permissions.require("network.connect")?;
                let Value::String(sql) = &args[1] else {
                    return Err(NativeError::type_error("SQL must be a string"));
                };
                if sql.len() > registered.options.max_sql_bytes {
                    return Err(connection.failure(
                        "exec",
                        None,
                        limit("database SQL byte limit exceeded"),
                    ));
                }
                connection
                    .call("exec", None, |_| true, |backend| backend.exec(context, sql))
                    .map(Value::Int)
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::rowCount",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("DbStatement")], NativeType::Int),
            move |context, args| {
                require_owner(context, &registered)?;
                let stmt = statement(&args[0], &registered)?;
                if stmt.closed.get() {
                    return Err(stmt.session.failure(
                        "rowCount",
                        Some(stmt),
                        state("database statement is closed"),
                    ));
                }
                stmt.session
                    .call(
                        "rowCount",
                        Some(stmt),
                        |_| false,
                        |backend| backend.row_count(stmt.id),
                    )
                    .map(Value::Int)
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::lastInsertId",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Object("DbConnection")],
                NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
            )
            .with_optional(vec![NativeType::OneOf(vec![
                NativeType::String,
                NativeType::Null,
            ])]),
            move |context, args| {
                require_owner(context, &registered)?;
                let connection = session(&args[0], &registered)?;
                let sequence = match args.get(1) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    _ => None,
                };
                connection.call(
                    "lastInsertId",
                    None,
                    |_| false,
                    |backend| backend.last_insert_id(context, sequence),
                )
            },
        ),
    )?;
    for (name, operation) in [
        ("Db::beginTransaction", TransactionOperation::Begin),
        ("Db::commit", TransactionOperation::Commit),
        ("Db::rollBack", TransactionOperation::RollBack),
        ("Db::inTransaction", TransactionOperation::Inspect),
    ] {
        let registered = registration.clone();
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::Object("DbConnection")],
                    if operation == TransactionOperation::Inspect {
                        NativeType::Bool
                    } else {
                        NativeType::Null
                    },
                ),
                move |context, args| {
                    require_owner(context, &registered)?;
                    let connection = session(&args[0], &registered)?;
                    context.check_cancelled()?;
                    context.permissions.require("network.connect")?;
                    let active = connection.call(
                        name.strip_prefix("Db::").unwrap_or(name),
                        None,
                        |_| operation != TransactionOperation::Inspect,
                        |backend| backend.transaction(context, operation),
                    )?;
                    Ok(if operation == TransactionOperation::Inspect {
                        Value::Bool(active)
                    } else {
                        Value::Null
                    })
                },
            ),
        )?;
    }
    let registered = registration.clone();
    runtime.register(
        "Db::prepare",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Object("DbConnection"), NativeType::String],
                NativeType::Object("DbStatement"),
            ),
            move |context, args| {
                require_owner(context, &registered)?;
                let connection = session(&args[0], &registered)?;
                context.check_cancelled()?;
                context.permissions.require("network.connect")?;
                let Value::String(sql) = &args[1] else {
                    return Err(NativeError::type_error("SQL must be a string"));
                };
                if sql.len() > registered.options.max_sql_bytes {
                    return Err(connection.failure(
                        "prepare",
                        None,
                        limit("database SQL byte limit exceeded"),
                    ));
                }
                if connection.statements.borrow().len()
                    >= registered.options.max_statements_per_connection
                {
                    return Err(connection.failure(
                        "prepare",
                        None,
                        limit("database statement limit exceeded"),
                    ));
                }
                let id = connection.call(
                    "prepare",
                    None,
                    |_| true,
                    |backend| backend.prepare(context, sql),
                )?;
                if let Err(error) = context.check_cancelled() {
                    drop(connection.release());
                    return Err(error);
                }
                if connection.statements.borrow().contains_key(&id) {
                    drop(connection.release());
                    return Err(connection.failure(
                        "prepare",
                        None,
                        state("backend reused a live statement identifier"),
                    ));
                }
                let shared = connection.shared()?;
                let statement = Rc::new(Statement {
                    session: shared,
                    id,
                    closed: Cell::new(false),
                    error: RefCell::new(None),
                });
                connection
                    .statements
                    .borrow_mut()
                    .insert(id, Rc::downgrade(&statement));
                Ok(Value::NativeObject(statement))
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::execute",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("DbStatement")], NativeType::Null)
                .with_optional(vec![NativeType::OneOf(vec![
                    NativeType::List,
                    NativeType::Map,
                ])]),
            move |context, args| {
                require_owner(context, &registered)?;
                let stmt = statement(&args[0], &registered)?;
                if stmt.closed.get() {
                    return Err(stmt.session.failure(
                        "execute",
                        Some(stmt),
                        state("database statement is closed"),
                    ));
                }
                context.check_cancelled()?;
                context.permissions.require("network.connect")?;
                let empty = Value::List(Vec::new());
                let params = args.get(1).unwrap_or(&empty);
                stmt.session.call(
                    "execute",
                    Some(stmt),
                    |_| true,
                    |backend| backend.execute(context, stmt.id, params),
                )?;
                Ok(Value::Null)
            },
        ),
    )?;
    for (name, returns) in [
        (
            "Db::fetch",
            NativeType::OneOf(vec![NativeType::Map, NativeType::Null]),
        ),
        ("Db::fetchAll", NativeType::List),
        (
            "Db::fetchColumn",
            NativeType::OneOf(vec![
                NativeType::Null,
                NativeType::Bool,
                NativeType::Int,
                NativeType::Float,
                NativeType::String,
                NativeType::Bytes,
            ]),
        ),
    ] {
        let registered = registration.clone();
        let mut signature = NativeSignature::new(vec![NativeType::Object("DbStatement")], returns);
        if name == "Db::fetchColumn" {
            signature = signature.with_optional(vec![NativeType::Int]);
        }
        runtime.register(
            name,
            NativeFunction::new(signature, move |context, args| {
                require_owner(context, &registered)?;
                let stmt = statement(&args[0], &registered)?;
                if stmt.closed.get() {
                    return Err(stmt.session.failure(
                        name.strip_prefix("Db::").unwrap_or(name),
                        Some(stmt),
                        state("database statement is closed"),
                    ));
                }
                context.check_cancelled()?;
                context.permissions.require("network.connect")?;
                stmt.session.call(
                    name.strip_prefix("Db::").unwrap_or(name),
                    Some(stmt),
                    |value| match name {
                        "Db::fetch" => !matches!(value, Value::Null),
                        "Db::fetchAll" => !matches!(value,Value::List(rows) if rows.is_empty()),
                        _ => true,
                    },
                    |backend| match name {
                        "Db::fetch" => backend.fetch(context, stmt.id),
                        "Db::fetchAll" => backend.fetch_all(context, stmt.id),
                        _ => {
                            let column = match args.get(1) {
                                Some(Value::Int(column)) => *column,
                                _ => 0,
                            };
                            backend.fetch_column(context, stmt.id, column)
                        }
                    },
                )
            }),
        )?;
    }
    let registered = registration.clone();
    runtime.register(
        "Db::closeCursor",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("DbStatement")], NativeType::Null),
            move |context, args| {
                require_owner(context, &registered)?;
                let stmt = statement(&args[0], &registered)?;
                if !stmt.closed.get() {
                    stmt.session.call(
                        "closeCursor",
                        Some(stmt),
                        |_| false,
                        |backend| backend.close_cursor(context, stmt.id),
                    )?;
                }
                Ok(Value::Null)
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::closeStatement",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("DbStatement")], NativeType::Null),
            move |context, args| {
                require_owner(context, &registered)?;
                let statement = statement(&args[0], &registered)?;
                if !statement.closed.get() {
                    let result = statement
                        .session
                        .backend
                        .borrow()
                        .as_ref()
                        .ok_or_else(|| state("database connection is closed"))
                        .and_then(|backend| backend.validate_close_statement(statement.id));
                    if let Err(error) = result {
                        return Err(statement.session.failure(
                            "closeStatement",
                            Some(statement),
                            error,
                        ));
                    }
                }
                if !statement.closed.replace(true) {
                    statement
                        .session
                        .statements
                        .borrow_mut()
                        .remove(&statement.id);
                    let (result, details) = match statement.session.backend.borrow_mut().as_mut() {
                        Some(backend) => {
                            backend.take_error_details();
                            let result = backend.close_statement(context, statement.id);
                            (result, backend.take_error_details())
                        }
                        None => (Ok(()), None),
                    };
                    statement.session.observe(
                        "closeStatement",
                        Some(statement),
                        |_| false,
                        &result,
                        details,
                    );
                    if let Err(error) = result {
                        // Do not free a slot while leaving an unusable server
                        // statement/session available for subsequent work.
                        drop(statement.session.release());
                        return Err(error);
                    }
                }
                Ok(Value::Null)
            },
        ),
    )?;
    let registered = registration.clone();
    runtime.register(
        "Db::errorInfo",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::OneOf(vec![
                    NativeType::Object("DbConnection"),
                    NativeType::Object("DbStatement"),
                ])],
                NativeType::OneOf(vec![NativeType::Map, NativeType::Null]),
            ),
            move |context, args| {
                require_owner(context, &registered)?;
                context.check_cancelled()?;
                let record = match &args[0] {
                    Value::NativeObject(object)
                        if object.as_any().is_some_and(|value| value.is::<Session>()) =>
                    {
                        session(&args[0], &registered)?.error.borrow().clone()
                    }
                    _ => statement(&args[0], &registered)?.error.borrow().clone(),
                };
                record.map_or(Ok(Value::Null), |record| {
                    record.value(context.max_value_bytes)
                })
            },
        ),
    )?;
    runtime.register(
        "Db::close",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Object("DbConnection")], NativeType::Null),
            move |context, args| {
                require_owner(context, &registration)?;
                let connection = session(&args[0], &registration)?;
                if let Some(mut backend) = connection.release() {
                    let result = backend.close(context);
                    connection.observe(
                        "close",
                        None,
                        |_| false,
                        &result,
                        backend.take_error_details(),
                    );
                    result?;
                }
                Ok(Value::Null)
            },
        ),
    )
}

#[cfg(test)]
mod tests;
