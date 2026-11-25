//! Server-reported transaction state through the client's public tracking API.
use crate::protocol::Failure;
use mysql::{Conn, prelude::Queryable, session_state_change::SessionStateChange};
use scr_db::TransactionOperation;
use scr_runtime::NativeError;

fn unknown() -> Failure {
    Failure {
        error: NativeError::new("DbError", "cannot establish MySQL transaction state"),
        fatal: true,
    }
}
#[derive(Default)]
pub(crate) struct Tracker {
    active: bool,
}
impl Tracker {
    fn capture(&mut self, connection: &Conn) -> Result<bool, Failure> {
        let mut found = false;
        for change in connection.session_state_changes().map_err(|_| unknown())? {
            if let SessionStateChange::TransactionState(state) =
                change.decode().map_err(|_| unknown())?
            {
                let bytes = state.as_bytes();
                if bytes.len() != 8 || !matches!(bytes[0], b'_' | b'T' | b'I') {
                    return Err(unknown());
                }
                self.active = bytes[0] != b'_';
                found = true;
            }
        }
        Ok(found)
    }
    pub fn refresh(&mut self, connection: &mut Conn) -> Result<(), Failure> {
        // Capture the preceding OK before the probe replaces it. ERR packets and
        // legacy EOF do not carry tracking data; DO flushes any pending changes.
        self.capture(connection)?;
        connection.query_drop("DO 0").map_err(|_| unknown())?;
        self.capture(connection)?;
        Ok(())
    }
    pub fn initialize(connection: &mut Conn) -> Result<(), Failure> {
        connection
            .query_drop("SET SESSION session_track_transaction_info='STATE'")
            .map_err(|_| unknown())?;
        let mut tracker = Self::default();
        // Verify actual negotiated tracking in both directions before publishing
        // the connection. These empty transactions have no application writes.
        connection
            .query_drop("START TRANSACTION")
            .map_err(|_| unknown())?;
        if !tracker.capture(connection)? || !tracker.active {
            return Err(unknown());
        }
        connection.query_drop("ROLLBACK").map_err(|_| unknown())?;
        if !tracker.capture(connection)? || tracker.active {
            return Err(unknown());
        }
        Ok(())
    }
    pub fn operation(
        &mut self,
        connection: &mut Conn,
        operation: TransactionOperation,
    ) -> Result<bool, Failure> {
        self.refresh(connection)?;
        if operation == TransactionOperation::Inspect {
            return Ok(self.active);
        }
        if self.active == (operation == TransactionOperation::Begin) {
            return Err(Failure {
                error: NativeError::new(
                    "DbStateError",
                    if self.active {
                        "nested transactions are not supported"
                    } else {
                        "no active MySQL transaction"
                    },
                ),
                fatal: false,
            });
        }
        let sql = match operation {
            TransactionOperation::Begin => "START TRANSACTION",
            TransactionOperation::Commit => "COMMIT",
            TransactionOperation::RollBack => "ROLLBACK",
            TransactionOperation::Inspect => unreachable!(),
        };
        if connection.query_drop(sql).is_err() {
            // A failed control request must never be retried. In particular a
            // missing commit acknowledgement cannot establish whether it committed.
            return Err(Failure {
                error: NativeError::new(
                    "DbError",
                    "MySQL transaction control failed; outcome is unknown",
                ),
                fatal: true,
            });
        }
        self.refresh(connection)?;
        if self.active != (operation == TransactionOperation::Begin) {
            return Err(unknown());
        }
        Ok(self.active)
    }
}
