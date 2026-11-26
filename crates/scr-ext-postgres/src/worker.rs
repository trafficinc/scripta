use crate::{
    decode::{self, Selection},
    protocol::{self, Message, Reply, Request},
    settings::{Attributes, Config, PostgresOptions},
    sql::{self, Mode, Parameters},
};
use scr_runtime::{NativeError, NativeResult, RuntimeContext};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) struct Worker {
    // The worker owns the connection. Dropping the last sender disconnects it;
    // Drop never joins a thread or performs blocking network I/O on the script.
    sender: Option<Sender<Message>>,
    stop: Arc<AtomicBool>,
    mode: Mode,
    plans: BTreeMap<u64, Plan>,
    active: Option<u64>,
    result_bytes: usize,
    pub last_error_details: Option<scr_db::ErrorDetails>,
    pub attributes: Attributes,
    pub policy: Arc<PostgresOptions>,
    pub retained_bytes: usize,
}
#[derive(Clone, Copy)]
enum ResultState {
    Prepared,
    Command,
    Cursor,
    Exhausted,
    Closed,
    Failed,
}
struct Plan {
    parameters: Parameters,
    bytes: usize,
    state: ResultState,
    columns: usize,
    result_limit: usize,
    kind: sql::Kind,
    affected: Option<u64>,
    types: Vec<u32>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
    }
}
impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConnection")
            .field("closed", &self.sender.is_none())
            .finish_non_exhaustive()
    }
}
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn permit(count: &Arc<AtomicUsize>, max: usize) -> NativeResult<Permit> {
    let mut live = count.load(Ordering::Acquire);
    loop {
        if live >= max {
            return Err(NativeError::new(
                "DbLimitError",
                "PostgreSQL worker limit exceeded",
            ));
        }
        match count.compare_exchange_weak(live, live + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Ok(Permit(count.clone())),
            Err(current) => live = current,
        }
    }
}
fn wait<T>(
    context: &RuntimeContext<'_>,
    response: Receiver<T>,
    timeout: Duration,
) -> NativeResult<T> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| NativeError::type_error("PostgreSQL timeout is too large"))?;
    loop {
        context.check_cancelled()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(NativeError::new(
                "DbTimeoutError",
                "PostgreSQL operation deadline exceeded",
            ));
        }
        match response.recv_timeout(remaining.min(Duration::from_millis(10))) {
            Ok(result) => {
                context.check_cancelled()?;
                if Instant::now() >= deadline {
                    return Err(NativeError::new(
                        "DbTimeoutError",
                        "PostgreSQL operation deadline exceeded",
                    ));
                }
                return Ok(result);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(NativeError::new(
                    "DbError",
                    "PostgreSQL worker disconnected",
                ));
            }
        }
    }
}
fn ca_snapshot(
    context: &RuntimeContext<'_>,
    config: &Config,
    policy: &PostgresOptions,
) -> NativeResult<Option<Vec<u8>>> {
    let Some(path) = &config.attributes.ca_path else {
        return Ok(None);
    };
    let mut source = policy.filesystem.open_read(context, path)?;
    let mut data = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        context.check_cancelled()?;
        let n = source
            .read(&mut buffer)
            .map_err(|_| NativeError::new("DbError", "cannot read PostgreSQL CA file"))?;
        if n == 0 {
            break;
        }
        if data.len().saturating_add(n) > policy.max_ca_bytes
            || context.max_value_bytes.is_some_and(|limit| {
                data.len()
                    .saturating_add(n)
                    .saturating_add(std::mem::size_of::<scr_runtime::Value>())
                    > limit
            })
        {
            return Err(NativeError::new(
                "DbLimitError",
                "PostgreSQL CA file exceeds byte limit",
            ));
        }
        data.extend_from_slice(&buffer[..n]);
    }
    if data.is_empty() {
        return Err(NativeError::new("DbError", "PostgreSQL CA file is empty"));
    }
    Ok(Some(data))
}
impl Worker {
    pub fn open(
        context: &RuntimeContext<'_>,
        config: Config,
        policy: Arc<PostgresOptions>,
        workers: &Arc<AtomicUsize>,
    ) -> NativeResult<Self> {
        let retained_bytes = config.host.len()
            + config.database.len()
            + config.user.len()
            + config.password.len()
            + config.attributes.ca_path.as_ref().map_or(0, String::len);
        if context.max_value_bytes.is_some_and(|limit| {
            retained_bytes.saturating_add(std::mem::size_of::<scr_runtime::Value>()) > limit
        }) {
            return Err(NativeError::new(
                "DbLimitError",
                "PostgreSQL connection payload exceeds value limit",
            ));
        }
        let snapshot = ca_snapshot(context, &config, &policy)?;
        context.check_cancelled()?;
        let permit = permit(workers, policy.max_workers)?;
        let attributes = config.attributes.clone();
        let connect_timeout = Duration::from_millis(attributes.connect_timeout_ms);
        let io_timeout = Duration::from_millis(policy.max_operation_timeout_ms);
        let packet_bytes = policy
            .max_parameter_bytes
            .max(policy.max_result_bytes)
            .max(1024 * 1024)
            .saturating_add(65536);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (sender, commands) = mpsc::channel::<Message>();
        let (ready, response) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("scripta-postgres".into())
            .spawn(move || {
                let opened = protocol::Session::open(
                    config,
                    snapshot.as_deref(),
                    packet_bytes,
                    io_timeout,
                    worker_stop.clone(),
                );
                match opened {
                    Ok(mut connection) => {
                        let mode = Mode;
                        if ready.send(Ok(mode)).is_err() {
                            return;
                        }
                        let acknowledgement =
                            protocol::serve(&mut connection, commands, worker_stop);
                        drop(connection);
                        drop(permit);
                        if let Some(acknowledgement) = acknowledgement {
                            let _ = acknowledgement.send(Ok(Reply::Done));
                        }
                    }
                    Err(error) => {
                        drop(permit);
                        let _ = ready.send(Err(error.error));
                    }
                }
            })
            .map_err(|_| NativeError::new("DbError", "cannot start PostgreSQL worker"))?;
        let mode = match wait(context, response, connect_timeout) {
            Ok(Ok(mode)) => mode,
            Ok(Err(error)) | Err(error) => {
                stop.store(true, Ordering::Release);
                return Err(error);
            }
        };
        Ok(Self {
            sender: Some(sender),
            attributes,
            policy,
            retained_bytes,
            stop,
            mode,
            plans: BTreeMap::new(),
            active: None,
            result_bytes: 0,
            last_error_details: None,
        })
    }
    fn break_connection(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
    }
    pub fn is_broken(&self) -> bool {
        self.sender.is_none()
    }
    pub fn payload(&self) -> usize {
        self.plans.values().fold(
            self.retained_bytes.saturating_add(self.result_bytes),
            |total, plan| total.saturating_add(plan.bytes),
        )
    }
    fn available(&self) -> NativeResult<()> {
        if self.active.is_some() {
            return Err(NativeError::new(
                "DbStateError",
                "close or exhaust the active cursor first",
            ));
        }
        if self.is_broken() {
            return Err(NativeError::new(
                "DbStateError",
                "PostgreSQL connection is broken",
            ));
        }
        Ok(())
    }
    fn rpc(&mut self, context: &RuntimeContext<'_>, request: Request) -> NativeResult<Reply> {
        self.last_error_details = None;
        if let Err(error) = context.check_cancelled() {
            self.break_connection();
            return Err(error);
        }
        let (response, receiver) = mpsc::sync_channel(1);
        let Some(sender) = &self.sender else {
            return Err(NativeError::new(
                "DbStateError",
                "PostgreSQL connection is broken",
            ));
        };
        if sender
            .send(Message {
                request,
                response,
                deadline: Instant::now()
                    .checked_add(self.attributes.operation_timeout())
                    .ok_or_else(|| NativeError::type_error("PostgreSQL timeout is too large"))?,
            })
            .is_err()
        {
            self.break_connection();
            self.last_error_details = Some(scr_db::ErrorDetails {
                category: scr_db::ErrorCategory::Connection,
                sql_state: None,
                driver_code: None,
            });
            return Err(NativeError::new(
                "DbError",
                "PostgreSQL worker disconnected",
            ));
        }
        match wait(context, receiver, self.attributes.operation_timeout()) {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(failure)) => {
                self.last_error_details = failure.details;
                if failure.fatal {
                    self.break_connection();
                }
                Err(failure.error)
            }
            Err(error) => {
                self.break_connection();
                if error.kind == "DbError" {
                    self.last_error_details = Some(scr_db::ErrorDetails {
                        category: scr_db::ErrorCategory::Connection,
                        sql_state: None,
                        driver_code: None,
                    });
                }
                Err(error)
            }
        }
    }
    pub fn set_attribute(&mut self, name: &str, value: &scr_runtime::Value) -> NativeResult<()> {
        self.available()?;
        self.attributes.set(name, value, &self.policy, false)
    }
    pub fn prepare(&mut self, context: &RuntimeContext<'_>, sql: &str) -> NativeResult<u64> {
        self.available()?;
        let parsed = sql::parse(sql, self.mode, self.policy.max_parameters, || {
            context.check_cancelled()
        })?;
        let names_bytes = parsed
            .parameters
            .payload()
            .saturating_add(std::mem::size_of::<Plan>());
        let expected = parsed.parameters.len();
        let reply = self.rpc(context, Request::Prepare(parsed.sql))?;
        let Reply::Prepared {
            id,
            bytes,
            parameters,
            types,
        } = reply
        else {
            self.break_connection();
            return Err(NativeError::new(
                "DbStateError",
                "unexpected prepare response",
            ));
        };
        let bytes = bytes.saturating_add(names_bytes);
        if parameters != expected {
            self.break_connection();
            return Err(NativeError::new(
                "DbStateError",
                "SQL parameter count disagrees with backend metadata",
            ));
        }
        if context.max_value_bytes.is_some_and(|limit| {
            self.payload()
                .saturating_add(bytes)
                .saturating_add(std::mem::size_of::<scr_runtime::Value>())
                > limit
        }) {
            self.break_connection();
            return Err(NativeError::new(
                "DbLimitError",
                "prepared statement metadata exceeds value limit",
            ));
        }
        self.plans.insert(
            id,
            Plan {
                parameters: parsed.parameters,
                types,
                kind: parsed.kind,
                affected: None,
                bytes,
                state: ResultState::Prepared,
                columns: 0,
                result_limit: self.attributes.max_result_bytes,
            },
        );
        Ok(id)
    }
    pub fn execute(
        &mut self,
        context: &RuntimeContext<'_>,
        id: u64,
        params: &scr_runtime::Value,
    ) -> NativeResult<()> {
        self.available()?;
        let plan = self
            .plans
            .get(&id)
            .ok_or_else(|| NativeError::new("DbStateError", "PostgreSQL statement is closed"))?;
        let params = sql::bind(&plan.parameters, params, &self.policy, context)?;
        if params
            .iter()
            .zip(&plan.types)
            .any(|(p, oid)| p.binary && *oid != 17)
        {
            return Err(NativeError::new(
                "DbTypeError",
                "bytes parameters require a PostgreSQL bytea target or explicit ::bytea cast",
            ));
        }
        let bytes = self
            .attributes
            .max_result_bytes
            .min(context.max_value_bytes.map_or(usize::MAX, |limit| {
                limit
                    .saturating_sub(self.payload())
                    .saturating_sub(std::mem::size_of::<scr_runtime::Value>())
            }));
        let limits = protocol::Limits {
            rows: self.attributes.max_rows,
            bytes,
            cell: self.policy.max_cell_bytes,
            deadline: Instant::now()
                .checked_add(self.attributes.operation_timeout())
                .ok_or_else(|| NativeError::type_error("PostgreSQL timeout is too large"))?,
        };
        self.result_bytes = 0;
        if let Some(plan) = self.plans.get_mut(&id) {
            plan.state = ResultState::Failed;
            plan.affected = None;
            plan.columns = 0;
            plan.result_limit = self.attributes.max_result_bytes;
        }
        let reply = self.rpc(context, Request::Execute { id, params, limits })?;
        let Reply::Executed {
            result,
            bytes,
            columns,
            affected,
        } = reply
        else {
            self.break_connection();
            return Err(NativeError::new(
                "DbStateError",
                "unexpected execute response",
            ));
        };
        self.active = result.then_some(id);
        self.result_bytes = bytes;
        if let Some(plan) = self.plans.get_mut(&id) {
            plan.state = if result {
                ResultState::Cursor
            } else {
                ResultState::Command
            };
            plan.columns = columns;
            plan.affected = Some(affected);
        }
        Ok(())
    }
    pub fn exec(&mut self, context: &RuntimeContext<'_>, sql: &str) -> NativeResult<i64> {
        self.available()?;
        let parsed = sql::parse_exec(sql, self.mode, || context.check_cancelled())?;
        let Reply::Command { affected } = self.rpc(context, Request::Exec(parsed.sql))? else {
            self.break_connection();
            return Err(NativeError::new("DbStateError", "unexpected exec response"));
        };
        count(if parsed.kind == sql::Kind::Ddl {
            0
        } else {
            affected
        })
    }
    pub fn row_count(&self, id: u64) -> NativeResult<i64> {
        let plan = self
            .plans
            .get(&id)
            .ok_or_else(|| NativeError::new("DbStateError", "PostgreSQL statement is closed"))?;
        let affected = plan.affected.ok_or_else(|| {
            NativeError::new("DbStateError", "statement has no successful execution")
        })?;
        if plan.kind == sql::Kind::Select {
            return Err(NativeError::new(
                "DbUnsupportedError",
                "rowCount is unavailable for SELECT; use COUNT(*)",
            ));
        }
        if plan.columns > 0 && matches!(plan.state, ResultState::Cursor) {
            return Err(NativeError::new(
                "DbStateError",
                "exhaust RETURNING rows before reading rowCount",
            ));
        }
        count(affected)
    }
    pub fn last_insert_id(
        &mut self,
        context: &RuntimeContext<'_>,
        sequence: Option<&str>,
    ) -> NativeResult<scr_runtime::Value> {
        self.available()?;
        let sequence = sequence.ok_or_else(|| {
            NativeError::new(
                "DbUnsupportedError",
                "PostgreSQL insert-ID lookup requires a sequence; prefer RETURNING",
            )
        })?;
        if sequence.is_empty()
            || sequence.contains('\0')
            || sequence.len() > self.policy.max_cell_bytes
        {
            return Err(NativeError::type_error("invalid PostgreSQL sequence name"));
        }
        context.permissions.require("network.connect")?;
        let Reply::InsertId(value) = self.rpc(context, Request::InsertId(sequence.into()))? else {
            self.break_connection();
            return Err(NativeError::new(
                "DbStateError",
                "unexpected sequence response",
            ));
        };
        Ok(scr_runtime::Value::String(value))
    }
    pub fn transaction(
        &mut self,
        context: &RuntimeContext<'_>,
        operation: scr_db::TransactionOperation,
    ) -> NativeResult<bool> {
        if operation != scr_db::TransactionOperation::Inspect {
            self.available()?;
        }
        let Reply::Transaction(active) = self.rpc(context, Request::Transaction(operation))? else {
            self.break_connection();
            return Err(NativeError::new(
                "DbStateError",
                "unexpected transaction response",
            ));
        };
        Ok(active)
    }
    pub fn validate_fetch(&self, id: u64) -> NativeResult<()> {
        let plan = self
            .plans
            .get(&id)
            .ok_or_else(|| NativeError::new("DbStateError", "PostgreSQL statement is closed"))?;
        if !matches!(plan.state, ResultState::Cursor | ResultState::Exhausted) {
            return Err(NativeError::new(
                "DbStateError",
                "PostgreSQL statement has no fetchable result",
            ));
        }
        Ok(())
    }
    pub fn fetch(
        &mut self,
        context: &RuntimeContext<'_>,
        id: u64,
        selection: Selection,
    ) -> NativeResult<scr_runtime::Value> {
        self.validate_fetch(id)?;
        let plan = self
            .plans
            .get(&id)
            .ok_or_else(|| NativeError::new("DbStateError", "PostgreSQL statement is closed"))?;
        if matches!(selection,Selection::Column(index) if index>=plan.columns) {
            return Err(NativeError::new(
                "IndexError",
                "PostgreSQL column index out of range",
            ));
        }
        if matches!(plan.state, ResultState::Exhausted) {
            return match selection {
                Selection::Row => Ok(scr_runtime::Value::Null),
                Selection::All => Ok(scr_runtime::Value::list(Vec::new())),
                Selection::Column(_) => Err(NativeError::new(
                    "DbEndOfResultsError",
                    "no more result rows",
                )),
            };
        }
        let result_limit = plan.result_limit;
        let deadline = Instant::now()
            .checked_add(self.attributes.operation_timeout())
            .ok_or_else(|| NativeError::type_error("PostgreSQL timeout is too large"))?;
        let outcome = (|| {
            let reply = self.rpc(
                context,
                Request::Fetch {
                    id,
                    all: matches!(selection, Selection::All),
                    deadline,
                },
            )?;
            let Reply::Fetched {
                rows,
                columns,
                bytes,
                exhausted,
            } = reply
            else {
                self.break_connection();
                return Err(NativeError::new(
                    "DbStateError",
                    "unexpected fetch response",
                ));
            };
            self.result_bytes = bytes;
            if exhausted {
                self.active = None;
                if let Some(plan) = self.plans.get_mut(&id) {
                    plan.state = ResultState::Exhausted;
                }
            }
            let budget = decode::Budget {
                bytes: result_limit.min(context.max_value_bytes.unwrap_or(usize::MAX)),
                cell: self.policy.max_cell_bytes,
            };
            decode::convert(&rows, &columns, selection, budget, || {
                context.check_cancelled()?;
                if Instant::now() >= deadline {
                    return Err(NativeError::new(
                        "DbTimeoutError",
                        "PostgreSQL fetch deadline exceeded",
                    ));
                }
                Ok(())
            })
        })();
        match outcome {
            Ok(value) => Ok(value),
            Err(error) if error.kind == "DbEndOfResultsError" => Err(error),
            Err(error) => {
                if matches!(error.kind.as_str(), "Interrupted" | "DbTimeoutError") {
                    self.break_connection();
                } else if self.active == Some(id) {
                    if let Err(cleanup) = self.close_cursor(context, id) {
                        if cleanup.kind == "Interrupted" {
                            return Err(cleanup);
                        }
                    }
                }
                self.active = None;
                self.result_bytes = 0;
                if let Some(plan) = self.plans.get_mut(&id) {
                    plan.state = ResultState::Failed;
                    plan.affected = None;
                }
                Err(error)
            }
        }
    }
    pub fn close_cursor(&mut self, context: &RuntimeContext<'_>, id: u64) -> NativeResult<()> {
        if !self.plans.contains_key(&id) {
            return Err(NativeError::new(
                "DbStateError",
                "PostgreSQL statement is closed",
            ));
        }
        if self.active == Some(id) {
            self.rpc(context, Request::CloseCursor(id))?;
            self.active = None;
            self.result_bytes = 0;
        }
        if let Some(plan) = self.plans.get_mut(&id) {
            if plan.columns > 0 && matches!(plan.state, ResultState::Cursor) {
                plan.affected = None;
            }
            plan.state = ResultState::Closed;
        }
        Ok(())
    }
    pub fn validate_close_statement(&self, id: u64) -> NativeResult<()> {
        if self.active.is_some_and(|active| active != id) {
            return Err(NativeError::new(
                "DbStateError",
                "another statement has an active cursor",
            ));
        }
        Ok(())
    }
    pub fn close_statement(&mut self, context: &RuntimeContext<'_>, id: u64) -> NativeResult<()> {
        self.validate_close_statement(id)?;
        self.rpc(context, Request::CloseStatement(id))?;
        self.plans.remove(&id);
        if self.active == Some(id) {
            self.active = None;
            self.result_bytes = 0;
        }
        Ok(())
    }
    pub fn close(&mut self, context: &RuntimeContext<'_>) -> NativeResult<()> {
        if self.sender.is_some() {
            self.rpc(context, Request::Close)?;
            self.sender.take();
        }
        Ok(())
    }
}

fn count(value: u64) -> NativeResult<i64> {
    i64::try_from(value).map_err(|_| {
        NativeError::new(
            "DbLimitError",
            "PostgreSQL affected-row count exceeds int range",
        )
    })
}

#[cfg(test)]
mod count_tests {
    use super::count;
    #[test]
    fn affected_counts_reject_overflow_without_lossy_conversion() {
        assert_eq!(count(i64::MAX as u64).unwrap(), i64::MAX);
        assert_eq!(count(i64::MAX as u64 + 1).unwrap_err().kind, "DbLimitError");
        assert_eq!(count(u64::MAX).unwrap_err().kind, "DbLimitError");
    }
}
