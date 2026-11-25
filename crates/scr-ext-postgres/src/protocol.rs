//! Worker requests use owned Send data; script values never cross the thread.
use crate::sql::Parameter;
pub(crate) use crate::wire::{Column, Row, Session};
use scr_db::TransactionOperation;
use scr_runtime::NativeError;
use std::{
    collections::{BTreeMap, VecDeque},
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
    pub details: Option<scr_db::ErrorDetails>,
}
pub(crate) fn failure(kind: &str, message: &str, fatal: bool) -> Failure {
    Failure {
        error: NativeError::new(kind, message),
        fatal,
        details: None,
    }
}
pub(crate) struct Limits {
    pub rows: usize,
    pub bytes: usize,
    pub cell: usize,
    pub deadline: Instant,
}
pub(crate) enum Request {
    Prepare(String),
    Execute {
        id: u64,
        params: Vec<Parameter>,
        limits: Limits,
    },
    Fetch {
        id: u64,
        all: bool,
        deadline: Instant,
    },
    Exec(String),
    InsertId(String),
    Transaction(TransactionOperation),
    CloseCursor(u64),
    CloseStatement(u64),
    Close,
}
pub(crate) enum Reply {
    Prepared {
        id: u64,
        bytes: usize,
        parameters: usize,
        types: Vec<u32>,
    },
    Executed {
        result: bool,
        bytes: usize,
        columns: usize,
        affected: u64,
    },
    Fetched {
        rows: Vec<Row>,
        columns: Arc<[Column]>,
        bytes: usize,
        exhausted: bool,
    },
    Command {
        affected: u64,
    },
    InsertId(String),
    Transaction(bool),
    Done,
}
pub(crate) struct Message {
    pub request: Request,
    pub response: SyncSender<Result<Reply, Failure>>,
    pub deadline: Instant,
}
struct Buffer {
    id: u64,
    rows: VecDeque<Row>,
    columns: Arc<[Column]>,
    bytes: usize,
}
fn name(id: u64) -> String {
    format!("scripta_{id}")
}
fn state(message: &str) -> Failure {
    failure("DbStateError", message, false)
}
pub(crate) fn serve(
    session: &mut Session,
    commands: Receiver<Message>,
    stop: Arc<AtomicBool>,
) -> Option<SyncSender<Result<Reply, Failure>>> {
    let mut plans = BTreeMap::<u64, Vec<u32>>::new();
    let mut next = 0u64;
    let mut active: Option<Buffer> = None;
    while let Ok(message) = commands.recv() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        session.set_deadline(message.deadline);
        if matches!(message.request, Request::Close) {
            if matches!(session.status, b'T' | b'E') {
                if let Err(error) = session.simple("ROLLBACK", None) {
                    let _ = message.response.send(Err(error));
                    return None;
                }
                if session.status != b'I' {
                    let _ = message.response.send(Err(failure(
                        "DbStateError",
                        "PostgreSQL rollback was not confirmed",
                        true,
                    )));
                    return None;
                }
            }
            return Some(message.response);
        }
        let result = (|| {
            Ok(match message.request {
                Request::Prepare(sql) => {
                    next = next.checked_add(1).ok_or_else(|| {
                        failure(
                            "DbLimitError",
                            "PostgreSQL statement identifier exhausted",
                            true,
                        )
                    })?;
                    let response = session.prepare(&name(next), &sql)?;
                    let types = response.parameters;
                    let bytes = sql
                        .len()
                        .saturating_add(response.bytes)
                        .saturating_add(types.len() * 2 * std::mem::size_of::<u32>());
                    plans.insert(next, types.clone());
                    Reply::Prepared {
                        id: next,
                        bytes,
                        parameters: types.len(),
                        types,
                    }
                }
                Request::Execute { id, params, limits } => {
                    let types = plans
                        .get(&id)
                        .ok_or_else(|| state("PostgreSQL statement is closed"))?;
                    if types.len() != params.len() {
                        return Err(state("PostgreSQL parameter count mismatch"));
                    }
                    let response = session.execute(&name(id), &params, &limits)?;
                    let columns = response.columns.len();
                    let result = columns > 0;
                    let bytes = response.bytes;
                    if result {
                        active = Some(Buffer {
                            id,
                            rows: response.rows.into(),
                            columns: response.columns.into(),
                            bytes,
                        });
                    }
                    Reply::Executed {
                        result,
                        bytes: if result { bytes } else { 0 },
                        columns,
                        affected: response.affected,
                    }
                }
                Request::Fetch { id, all, deadline } => {
                    if Instant::now() >= deadline {
                        return Err(failure(
                            "DbTimeoutError",
                            "PostgreSQL fetch deadline exceeded",
                            true,
                        ));
                    }
                    let buffer = active
                        .as_mut()
                        .filter(|b| b.id == id)
                        .ok_or_else(|| state("PostgreSQL cursor is not active"))?;
                    let mut rows = Vec::new();
                    let count = if all {
                        buffer.rows.len()
                    } else {
                        1.min(buffer.rows.len())
                    };
                    for _ in 0..count {
                        if stop.load(Ordering::Acquire) {
                            return Err(failure("Interrupted", "PostgreSQL fetch cancelled", true));
                        }
                        let row = buffer
                            .rows
                            .pop_front()
                            .ok_or_else(|| state("PostgreSQL cursor is empty"))?;
                        let bytes = row.iter().fold(std::mem::size_of::<Row>(), |n, v| {
                            n.saturating_add(std::mem::size_of::<Option<Vec<u8>>>())
                                .saturating_add(v.as_ref().map_or(0, Vec::len))
                        });
                        buffer.bytes = buffer.bytes.saturating_sub(bytes);
                        rows.push(row);
                    }
                    let exhausted = buffer.rows.is_empty();
                    let columns = buffer.columns.clone();
                    let bytes = if exhausted { 0 } else { buffer.bytes };
                    if exhausted {
                        active = None;
                    }
                    Reply::Fetched {
                        rows,
                        columns,
                        bytes,
                        exhausted,
                    }
                }
                Request::Exec(sql) => {
                    let limits = Limits {
                        rows: 0,
                        bytes: 0,
                        cell: 0,
                        deadline: message.deadline,
                    };
                    let response = session.simple(&sql, Some(&limits))?;
                    Reply::Command {
                        affected: response.affected,
                    }
                }
                Request::InsertId(sequence) => {
                    // Reserved name never collides with script statement identifiers.
                    session.prepare("scripta_sequence", "SELECT currval($1::regclass)")?;
                    let limits = Limits {
                        rows: 1,
                        bytes: 4096,
                        cell: 128,
                        deadline: message.deadline,
                    };
                    let lookup = session.execute(
                        "scripta_sequence",
                        &[Parameter {
                            data: Some(sequence.into_bytes()),
                            binary: false,
                        }],
                        &limits,
                    );
                    let cleanup = session.close_statement("scripta_sequence");
                    let response = lookup?;
                    cleanup?;
                    let value = response
                        .rows
                        .first()
                        .and_then(|r| r.first())
                        .and_then(|v| v.as_ref())
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .ok_or_else(|| state("PostgreSQL sequence result is invalid"))?;
                    let value = value
                        .parse::<i64>()
                        .map_err(|_| state("PostgreSQL sequence result is invalid"))?;
                    Reply::InsertId(value.to_string())
                }
                Request::Transaction(operation) => {
                    // An empty query obtains fresh authoritative ReadyForQuery status,
                    // including an aborted transaction, without mutating the session.
                    session.simple("", None)?;
                    match operation {
                        TransactionOperation::Inspect => {}
                        TransactionOperation::Begin if session.status == b'I' => {
                            session.simple("BEGIN", None)?;
                            if session.status != b'T' {
                                return Err(failure(
                                    "DbStateError",
                                    "PostgreSQL did not begin a transaction",
                                    true,
                                ));
                            }
                        }
                        TransactionOperation::Commit if session.status == b'T' => {
                            session.simple("COMMIT", None)?;
                            if session.status != b'I' {
                                return Err(failure(
                                    "DbStateError",
                                    "PostgreSQL did not complete commit",
                                    true,
                                ));
                            }
                        }
                        TransactionOperation::RollBack if matches!(session.status, b'T' | b'E') => {
                            session.simple("ROLLBACK", None)?;
                            if session.status != b'I' {
                                return Err(failure(
                                    "DbStateError",
                                    "PostgreSQL did not complete rollback",
                                    true,
                                ));
                            }
                        }
                        TransactionOperation::Commit if session.status == b'E' => {
                            return Err(state(
                                "PostgreSQL transaction is aborted; roll back before continuing",
                            ));
                        }
                        _ => {
                            return Err(state(
                                "PostgreSQL transaction operation is invalid in the current state",
                            ));
                        }
                    }
                    Reply::Transaction(session.status != b'I')
                }
                Request::CloseCursor(id) => {
                    if active.as_ref().is_some_and(|b| b.id == id) {
                        active = None;
                    }
                    Reply::Done
                }
                Request::CloseStatement(id) => {
                    if active.as_ref().is_some_and(|b| b.id == id) {
                        active = None;
                    }
                    if plans.remove(&id).is_some() {
                        session.close_statement(&name(id))?;
                    }
                    Reply::Done
                }
                Request::Close => unreachable!(),
            })
        })();
        let result = result.map_err(|mut error: Failure| {
            if error.fatal && error.error.kind == "DbError" {
                error.error.message = "PostgreSQL connection failed; operation outcome is unknown and must not be retried automatically".into();
            }
            error
        });
        let fatal = result.as_ref().is_err_and(|error| error.fatal);
        if message.response.send(result).is_err() || fatal {
            break;
        }
    }
    None
}
