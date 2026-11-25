//! Verified-TLS PostgreSQL connections for the shared Db API.
//! Native execution, typed fetching and transactions; optional CLI mysql feature.
mod decode;
mod protocol;
mod settings;
mod sql;
mod wire;
mod worker;
use scr_db::{ConnectRequest, Connection, Driver, DriverInfo};
use scr_runtime::{NativeResult, RuntimeContext, Value};
pub use settings::{Destination, PostgresOptions};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicUsize},
};

#[derive(Debug)]
struct PostgresDriver {
    options: Arc<PostgresOptions>,
    workers: Arc<AtomicUsize>,
}
impl Driver for PostgresDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            name: "pgsql",
            version: env!("CARGO_PKG_VERSION"),
            capabilities: &[
                "tls.verifyIdentity",
                "prepare.native",
                "parameters.positional",
                "parameters.named",
                "fetch.assoc",
                "fetch.column",
                "exec",
                "rowCount",
                "lastInsertId.sequence",
                "dml.returning",
                "transactions",
                "transactions.state",
                "errorInfo",
            ],
        }
    }
    fn connect(
        &self,
        context: &mut RuntimeContext<'_>,
        request: ConnectRequest<'_>,
    ) -> NativeResult<Box<dyn Connection>> {
        context.check_cancelled()?;
        context.permissions.require("network.connect")?;
        let config = settings::Config::parse(request, &self.options)?;
        Ok(Box::new(worker::Worker::open(
            context,
            config,
            self.options.clone(),
            &self.workers,
        )?))
    }
}
impl Connection for worker::Worker {
    fn take_error_details(&mut self) -> Option<scr_db::ErrorDetails> {
        self.last_error_details.take()
    }
    fn exec(&mut self, context: &mut RuntimeContext<'_>, sql: &str) -> NativeResult<i64> {
        self.exec(context, sql)
    }
    fn row_count(&self, id: u64) -> NativeResult<i64> {
        self.row_count(id)
    }
    fn last_insert_id(
        &mut self,
        context: &mut RuntimeContext<'_>,
        sequence: Option<&str>,
    ) -> NativeResult {
        self.last_insert_id(context, sequence)
    }
    fn transaction(
        &mut self,
        context: &mut RuntimeContext<'_>,
        operation: scr_db::TransactionOperation,
    ) -> NativeResult<bool> {
        self.transaction(context, operation)
    }
    fn payload_bytes(&self) -> usize {
        self.payload()
    }
    fn get_attribute(&self, name: &str) -> NativeResult {
        self.attributes.get(name)
    }
    fn set_attribute(&mut self, name: &str, value: &Value) -> NativeResult<()> {
        self.set_attribute(name, value)
    }
    fn is_broken(&self) -> bool {
        self.is_broken()
    }
    fn prepare(&mut self, context: &mut RuntimeContext<'_>, sql: &str) -> NativeResult<u64> {
        self.prepare(context, sql)
    }
    fn execute(
        &mut self,
        context: &mut RuntimeContext<'_>,
        id: u64,
        params: &Value,
    ) -> NativeResult<()> {
        self.execute(context, id, params)
    }
    fn close_cursor(&mut self, context: &mut RuntimeContext<'_>, id: u64) -> NativeResult<()> {
        self.close_cursor(context, id)
    }
    fn fetch(&mut self, context: &mut RuntimeContext<'_>, id: u64) -> NativeResult {
        self.fetch(context, id, decode::Selection::Row)
    }
    fn fetch_all(&mut self, context: &mut RuntimeContext<'_>, id: u64) -> NativeResult {
        self.fetch(context, id, decode::Selection::All)
    }
    fn fetch_column(
        &mut self,
        context: &mut RuntimeContext<'_>,
        id: u64,
        column: i64,
    ) -> NativeResult {
        self.validate_fetch(id)?;
        let column = usize::try_from(column).map_err(|_| {
            scr_runtime::NativeError::new("IndexError", "PostgreSQL column index out of range")
        })?;
        self.fetch(context, id, decode::Selection::Column(column))
    }
    fn validate_close_statement(&self, id: u64) -> NativeResult<()> {
        self.validate_close_statement(id)
    }
    fn close_statement(&mut self, context: &mut RuntimeContext<'_>, id: u64) -> NativeResult<()> {
        self.close_statement(context, id)
    }
    fn close(&mut self, context: &mut RuntimeContext<'_>) -> NativeResult<()> {
        self.close(context)
    }
}

/// Create an I/O-free descriptor using default host limits and unrestricted destinations.
pub fn driver() -> Rc<dyn Driver> {
    Rc::new(PostgresDriver {
        options: Arc::new(PostgresOptions::default()),
        workers: Arc::new(AtomicUsize::new(0)),
    })
}
/// Validate host limits, destination grants and filesystem roots without connecting.
pub fn driver_with_options(options: PostgresOptions) -> NativeResult<Rc<dyn Driver>> {
    Ok(Rc::new(PostgresDriver {
        options: Arc::new(options.prepare()?),
        workers: Arc::new(AtomicUsize::new(0)),
    }))
}

#[cfg(test)]
mod tests;
