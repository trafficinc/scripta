//! Worker-owned statements and bounded, unread result buffers.
use crate::sql::Mode;
use mysql::{Conn, Statement, prelude::Queryable};
use scr_runtime::{NativeError, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender},
    },
    time::Instant,
};

pub(crate) struct Failure {
    pub error: NativeError,
    pub fatal: bool,
}
pub(crate) enum Reply {
    Prepared {
        id: u64,
        bytes: usize,
        parameters: usize,
    },
    Executed {
        result: bool,
        bytes: usize,
        columns: usize,
    },
    Fetched {
        rows: Vec<mysql::Row>,
        columns: Arc<[mysql::Column]>,
        bytes: usize,
        exhausted: bool,
    },
    Done,
}
pub(crate) enum Request {
    Prepare(String),
    Execute {
        id: u64,
        params: Vec<mysql::Value>,
        limits: Limits,
    },
    Fetch {
        id: u64,
        all: bool,
        deadline: Instant,
    },
    CloseCursor(u64),
    CloseStatement(u64),
    Close,
}
pub(crate) struct Message {
    pub request: Request,
    pub response: SyncSender<Result<Reply, Failure>>,
}
pub(crate) struct Limits {
    pub rows: usize,
    pub bytes: usize,
    pub cell: usize,
    pub deadline: Instant,
}
struct Buffer {
    rows: VecDeque<(mysql::Row, usize)>,
    columns: Arc<[mysql::Column]>,
    bytes: usize,
}
fn failure(kind: &str, message: &str, fatal: bool) -> Failure {
    Failure {
        error: NativeError::new(kind, message),
        fatal,
    }
}
fn backend(error: mysql::Error) -> Failure {
    let fatal = error.is_connectivity_error();
    let error = match error {
        mysql::Error::MySqlError(error) => {
            let state = if error.state.len() == 5
                && error.state.bytes().all(|b| b.is_ascii_alphanumeric())
            {
                error.state
            } else {
                "unknown".into()
            };
            NativeError::new(
                "DbError",
                format!(
                    "MySQL statement failed (SQLSTATE {state}, code {})",
                    error.code
                ),
            )
        }
        other => super::worker::connection_error(other),
    };
    Failure { error, fatal }
}
pub(crate) fn mode(connection: &mut Conn) -> mysql::Result<Mode> {
    let modes: Option<String> = connection.query_first("SELECT @@SESSION.sql_mode")?;
    let modes = modes.unwrap_or_default();
    Ok(Mode {
        no_backslash: modes.split(',').any(|m| m == "NO_BACKSLASH_ESCAPES"),
        ansi_quotes: modes.split(',').any(|m| m == "ANSI_QUOTES"),
    })
}
fn metadata(statement: &Statement) -> usize {
    statement
        .params()
        .iter()
        .chain(statement.columns().iter())
        .fold(0usize, |total, column| {
            total
                .saturating_add(mem::size_of::<mysql::Column>())
                .saturating_add(column.schema_ref().len())
                .saturating_add(column.table_ref().len())
                .saturating_add(column.org_table_ref().len())
                .saturating_add(column.name_ref().len())
                .saturating_add(column.org_name_ref().len())
        })
}
fn stopped(stop: &AtomicBool, deadline: Instant) -> Result<(), Failure> {
    if stop.load(Ordering::Acquire) {
        return Err(failure("Interrupted", "execution interrupted", true));
    }
    if Instant::now() >= deadline {
        return Err(failure(
            "DbTimeoutError",
            "MySQL operation deadline exceeded",
            true,
        ));
    }
    Ok(())
}
fn execute(
    connection: &mut Conn,
    statement: &Statement,
    params: Vec<mysql::Value>,
    limits: Limits,
    stop: &AtomicBool,
    response: SyncSender<Result<Reply, Failure>>,
    active: &mut Option<(u64, Buffer)>,
) -> bool {
    *active = None;
    let mut result = match connection.exec_iter(statement, mysql::Params::Positional(params)) {
        Ok(result) => result,
        Err(error) => {
            let error = backend(error);
            let reusable = !error.fatal;
            let _ = response.send(Err(error));
            return reusable;
        }
    };
    let has_columns = !result.columns().as_ref().is_empty();
    let columns = result.columns();
    let key_bytes = columns
        .as_ref()
        .iter()
        .map(|c| c.name_ref().len())
        .sum::<usize>();
    let row_slots = mem::size_of::<Value>()
        .saturating_mul(columns.as_ref().len().saturating_mul(2).saturating_add(2));
    let mut bytes = columns
        .as_ref()
        .iter()
        .fold(mem::size_of::<Value>(), |total, column| {
            total
                .saturating_add(mem::size_of::<mysql::Column>())
                .saturating_add(column.schema_ref().len())
                .saturating_add(column.table_ref().len())
                .saturating_add(column.org_table_ref().len())
                .saturating_add(column.name_ref().len())
                .saturating_add(column.org_name_ref().len())
        });
    if has_columns && bytes > limits.bytes {
        let _ = response.send(Err(failure(
            "DbLimitError",
            "MySQL result metadata exceeds byte limit",
            true,
        )));
        return false;
    }
    let column_count = columns.as_ref().len();
    let mut owned_columns = Vec::new();
    if owned_columns.try_reserve_exact(column_count).is_err() {
        let _ = response.send(Err(failure(
            "DbLimitError",
            "MySQL metadata allocation failed",
            true,
        )));
        return false;
    }
    owned_columns.extend(columns.as_ref().iter().cloned());
    let owned_columns: Arc<[mysql::Column]> = owned_columns.into();
    let mut rows = Vec::new();
    loop {
        let item = (|| {
            stopped(stop, limits.deadline)?;
            if has_columns && bytes > limits.bytes {
                return Err(failure(
                    "DbLimitError",
                    "MySQL result metadata exceeds byte limit",
                    true,
                ));
            }
            let Some(row) = result.next() else {
                return Ok(None);
            };
            let row = row.map_err(backend)?;
            stopped(stop, limits.deadline)?;
            if rows.len() >= limits.rows {
                return Err(failure(
                    "DbLimitError",
                    "MySQL result row limit exceeded",
                    true,
                ));
            }
            let mut size = row_slots.saturating_add(key_bytes);
            for index in 0..row.len() {
                if index % 64 == 0 {
                    stopped(stop, limits.deadline)?;
                }
                let value = row
                    .as_ref(index)
                    .ok_or_else(|| failure("DbStateError", "incomplete MySQL result row", true))?;
                let value_bytes = match value {
                    mysql::Value::Bytes(bytes) => bytes.len(),
                    _ => 0,
                };
                if value_bytes > limits.cell {
                    return Err(failure(
                        "DbLimitError",
                        "MySQL result cell limit exceeded",
                        true,
                    ));
                }
                size = size
                    .checked_add(value_bytes)
                    .ok_or_else(|| failure("DbLimitError", "MySQL result byte overflow", true))?;
            }
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| failure("DbLimitError", "MySQL result byte overflow", true))?;
            if bytes > limits.bytes {
                return Err(failure(
                    "DbLimitError",
                    "MySQL result byte limit exceeded",
                    true,
                ));
            }
            rows.try_reserve(1)
                .map_err(|_| failure("DbLimitError", "MySQL result allocation failed", true))?;
            Ok(Some((row, size)))
        })();
        match item {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => break,
            Err(mut error) => {
                // Even a server error encountered while reading a result may
                // leave unread protocol data. Quarantine this session. Send the
                // failure before the client's automatic Drop drain can block.
                error.fatal = true;
                let _ = response.send(Err(error));
                return false;
            }
        }
    }
    drop(result); // the single supported result is already consumed
    if has_columns {
        *active = Some((
            u64::from(statement.id()),
            Buffer {
                rows: rows.into(),
                columns: owned_columns,
                bytes,
            },
        ));
    }
    response
        .send(Ok(Reply::Executed {
            result: has_columns,
            bytes: if has_columns { bytes } else { 0 },
            columns: column_count,
        }))
        .is_ok()
}

fn fetch(
    active: &mut Option<(u64, Buffer)>,
    id: u64,
    all: bool,
    deadline: Instant,
    stop: &AtomicBool,
) -> Result<Reply, Failure> {
    let Some((owner, mut buffer)) = active.take() else {
        return Err(failure("DbStateError", "MySQL cursor is not active", false));
    };
    if owner != id {
        *active = Some((owner, buffer));
        return Err(failure(
            "DbStateError",
            "another MySQL cursor is active",
            false,
        ));
    }
    let count = if all {
        buffer.rows.len()
    } else {
        buffer.rows.len().min(1)
    };
    let mut rows = Vec::new();
    rows.try_reserve_exact(count)
        .map_err(|_| failure("DbLimitError", "MySQL fetch allocation failed", false))?;
    for _ in 0..count {
        stopped(stop, deadline)?;
        let (row, bytes) = buffer
            .rows
            .pop_front()
            .ok_or_else(|| failure("DbStateError", "incomplete MySQL buffer", false))?;
        buffer.bytes = buffer.bytes.saturating_sub(bytes);
        rows.push(row);
    }
    let columns = buffer.columns.clone();
    let exhausted = buffer.rows.is_empty();
    let bytes = if exhausted { 0 } else { buffer.bytes };
    if !exhausted {
        *active = Some((owner, buffer));
    }
    Ok(Reply::Fetched {
        rows,
        columns,
        bytes,
        exhausted,
    })
}

/// Close acknowledgement is sent only after the caller drops Conn and its permit.
pub(crate) fn serve(
    connection: &mut Conn,
    commands: Receiver<Message>,
    stop: Arc<AtomicBool>,
) -> Option<SyncSender<Result<Reply, Failure>>> {
    let mut statements = BTreeMap::<u64, Statement>::new();
    let mut active: Option<(u64, Buffer)> = None;
    while let Ok(message) = commands.recv() {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        let result = match message.request {
            Request::Prepare(sql) => match connection.prep(sql) {
                Ok(statement) => {
                    let id = u64::from(statement.id());
                    let bytes = metadata(&statement);
                    statements.insert(id, statement);
                    Ok(Reply::Prepared {
                        id,
                        bytes,
                        parameters: statements[&id].num_params() as usize,
                    })
                }
                Err(error) => Err(backend(error)),
            },
            Request::Execute { id, params, limits } => {
                let Some(statement) = statements.get(&id) else {
                    let _ = message.response.send(Err(failure(
                        "DbStateError",
                        "MySQL statement is closed",
                        false,
                    )));
                    continue;
                };
                if !execute(
                    connection,
                    statement,
                    params,
                    limits,
                    &stop,
                    message.response,
                    &mut active,
                ) {
                    return None;
                }
                continue;
            }
            Request::Fetch { id, all, deadline } => fetch(&mut active, id, all, deadline, &stop),
            Request::CloseCursor(id) => {
                if active.as_ref().is_some_and(|(active, _)| *active == id) {
                    active = None;
                }
                Ok(Reply::Done)
            }
            Request::CloseStatement(id) => {
                if active.as_ref().is_some_and(|(active, _)| *active == id) {
                    active = None;
                }
                match statements.remove(&id) {
                    Some(statement) => connection
                        .close(statement)
                        .map(|_| Reply::Done)
                        .map_err(backend),
                    None => Err(failure("DbStateError", "MySQL statement is closed", false)),
                }
            }
            Request::Close => return Some(message.response),
        };
        let fatal = result.as_ref().err().is_some_and(|e| e.fatal);
        if message.response.send(result).is_err() || fatal {
            return None;
        }
    }
    None
}
