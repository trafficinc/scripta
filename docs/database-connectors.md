# Database connector development

The shared database interface is under development. Embedding hosts can open and
close verified-TLS MySQL connections, inspect/configure attributes, and execute
native prepared statements with positional or named values, fetch typed results,
inspect execution metadata, and control transactions. CLI feature wiring remains
a later stage. The standard CLI still exposes only the
existing [SQLite API](sqlite.md); it does not register `Db::` or a MySQL feature.
Installing a script package cannot enable a compiled-out native driver.

## Crate boundaries

`scr-db` depends only on `scr-runtime`. It owns typed `Db::` registration, opaque
connection and statement handles, ownership checks, and shared host limits.
It contains no SQL protocol, TLS implementation, or concrete database driver.
Register all enabled driver descriptors together, once per runtime.

`scr-ext-mysql` depends on scr-db, scr-runtime, the Rust mysql client, tempfile for
private CA snapshots, and scr-stdlib for its existing filesystem policy. Concrete
network and database work stays outside the runtime and shared API. Future
PostgreSQL support belongs in a separate driver crate. Redis and other non-SQL
connectors do not implement this interface.

## Native registration and connection API

An embedding host composes descriptors through the public native interface:

```rust,ignore
scr_db::register(&mut runtime, vec![scr_ext_mysql::driver()])?;
```

Use `scr_ext_mysql::driver_with_options(MysqlOptions)` for host destination grants,
filesystem roots, timeouts, worker ceilings, and result-limit ceilings. Clone the
same host filesystem policy used for standard-library registration when both
interfaces should have the same roots. Descriptor creation validates settings;
registration performs no network I/O. On registration failure, discard the
partially initialized runtime.

| Available call | Behavior |
| --- | --- |
| `Db::connect(dsn, user, password, options = [])` | Open a DbConnection using a registered driver; credentials must be strings, options a string-keyed map or an empty list |
| `Db::getAttribute(db, name)` | Return a supported typed attribute |
| `Db::setAttribute(db, name, value)` | Apply a supported mutable setting; return null |
| `Db::prepare(db, sql)` | Prepare one supported SQL statement immediately on the server; return a DbStatement |
| `Db::execute(stmt, params = [])` | Bind a fresh list/map and execute; return null or throw |
| `Db::exec(db, sql)` | Execute one parameterless non-result DML or supported DDL statement; return affected rows as int |
| `Db::rowCount(stmt)` | Return affected rows from the latest successful DML execution; unsupported for SELECT |
| `Db::lastInsertId(db, sequence = null)` | Return this connection's latest accepted execute/exec generated ID as an exact string, or null; MySQL rejects sequence names |
| `Db::beginTransaction(db)` | Begin an explicit transaction; nesting raises DbStateError |
| `Db::commit(db)` | Commit an active transaction; an idle connection raises DbStateError |
| `Db::rollBack(db)` | Roll back an active transaction; an idle connection raises DbStateError |
| `Db::inTransaction(db)` | Query server-derived transaction state as bool |
| `Db::fetch(stmt)` | Consume the next associative row; return null at exhaustion |
| `Db::fetchAll(stmt)` | Consume all remaining rows into a list; return an empty list at exhaustion |
| `Db::fetchColumn(stmt, column = 0)` | Consume the next row and return its zero-based column; throw DbEndOfResultsError at exhaustion |
| `Db::closeCursor(stmt)` | Release unread buffered results, preserving the statement for reuse; repeated calls succeed |
| `Db::closeStatement(stmt)` | Release a statement and cursor; repeated calls succeed |
| `Db::close(db)` | Disconnect, invalidate dependent handles, and return null; repeated close succeeds |

Names in a DSN select an explicitly registered driver; there is no dynamic loading.
Missing drivers raise DbUnsupportedError. Structured Db::errorInfo inspection
remains planned; existing native error kinds and sanitized messages are available.

A MySQL DSN uses `mysql:host=localhost;port=3306;dbname=app;charset=utf8mb4`.
Host and dbname are required; port defaults to 3306 and charset to utf8mb4.
Only utf8mb4 and TCP are supported initially. Accept IPv4, bracketed IPv6, and
ASCII DNS names, normalizing DNS case. Reject duplicate/unknown/empty fields,
NULs, invalid ports, and credentials inside the DSN. DSNs are limited to 16 KiB;
user/password are limited to 4096 UTF-8 bytes each, with a nonempty user.
Semicolon escapes and trailing empty fields are not accepted.

### Attributes and host policy

| Name | Default | Rules |
| --- | --- | --- |
| errorMode | exception | Only exception mode |
| fetchMode | assoc | Only associative fetching |
| emulatePrepares | false | True is unsupported |
| connectTimeoutMs | 5000 | Positive int, bounded by host ceiling; connection-only |
| operationTimeoutMs | 30000 | Positive int, bounded by host ceiling; mutable |
| tlsMode | verifyIdentity | Only verified certificates and hostnames; connection-only |
| tlsCaFile | null | Optional CA file; connection-only |
| mysqlFoundRows | false | Bool; connection-only |
| maxRows | 10000 | Positive int, bounded by host ceiling; mutable |
| maxResultBytes | 8388608 | Positive int, bounded by host ceiling; mutable |
| driverName, driverVersion | mysql, connector API version | Read-only |

A reduced host ceiling also reduces the default. Incorrect types raise TypeError;
unsupported values/names raise DbUnsupportedError; attempts to change connection-only
settings raise DbStateError; attempts to exceed host ceilings raise DbLimitError.
A rejected change preserves the previous value. Mutable result settings are
snapshotted for each accepted execution, including subsequent fetches.

TCP database I/O requires network.connect. `MysqlOptions.destinations = None`
grants all destinations; `Some([])` denies all, and a nonempty list grants exact
normalized hostname/IP and port pairs. A hostname grant trusts that hostname's
DNS resolution; it does not restrict resolved addresses to an IP subnet.
Cleanup and local attribute inspection do not require a new network grant.

Custom CA reads require filesystem.read and use the host's prepared
FilesystemPolicy through its public open_read method. The same working-directory,
root, regular-file, and symlink checks apply as File::read. Read the source in
bounded chunks, applying cancellation and runtime value limits, with a default
1 MiB CA ceiling. Copy approved bytes into a private temporary file for the client
library; it never rereads a potentially changed script-supplied source. This
internal snapshot does not require script filesystem.write permission and is
removed by RAII. TLS certificate/hostname checks cannot be disabled.

### Prepared SQL and binding

Prepare reaches the server immediately using its native binary prepared-statement
protocol. It does not execute the statement. Execute always binds new values and
never interpolates them into SQL. Positional `?` markers require a list; named
`:Name` markers require a string-keyed map without colons in keys. Names are
case-sensitive ASCII identifiers. Reject mixed styles, repeated named markers,
missing/extra values, wrong key types, and unsupported value types before execution.
Use two distinct names when binding the same value twice. An omitted params list
is valid only for a statement with no markers.

Bind null, bool, int, finite float, string, and bytes. Bool uses the backend's
integer representation; strings and bytes use length-delimited protocol buffers,
with no quoting or UTF-8 decoding of bytes. Database column conversion rules still
apply. Collections, native objects, callables, and non-finite numbers cannot be
bound. Identifiers and SQL syntax cannot be supplied through value markers.

The initial prepared subset is SELECT, INSERT, UPDATE, DELETE, and REPLACE.
Exec accepts parameterless INSERT, UPDATE, DELETE and REPLACE, plus CREATE/ALTER/DROP
TABLE, CREATE [UNIQUE] INDEX and DROP INDEX. Exec rejects SELECT and parameter
markers before reaching the server. Always bind untrusted values through prepared
statements; exec does not interpolate or escape values for callers. Reject stacked statements,
session variables/control, SELECT INTO, stored procedure calls, RETURNING, executable
comments, nested comments, and optimizer hints. Ordinary comments, trailing whitespace
and one trailing semicolon are accepted. Other valid MySQL forms are not promised
by this conservative subset. Use database credentials with appropriate server
privileges; the scanner does not make arbitrary stored functions read-only.

The scanner respects strings, quoted identifiers, doubled quotes, backslash rules,
Unicode and explicit LF/CRLF line endings. It reads NO_BACKSLASH_ESCAPES and
ANSI_QUOTES from the actual session at connection startup. Session SQL-mode changes
are not exposed. The mysql client's own placeholder parser must leave normalized
SQL unchanged; incompatible quoting raises DbUnsupportedError before native prepare.
Server parameter counts must agree with the scanner before a handle is published.

The host can set max_parameters (default 1024, at most 65535), max_cell_bytes
(default 1 MiB), and max_parameter_bytes (default 8 MiB). Total parameter bytes
include logical value slots plus byte/string payload. Validate limits before copying
values into worker requests. SQL text defaults to the shared 1 MiB ceiling. Native
prepared metadata and named-marker storage count toward retained logical payload.

### Result fetching and state

Execute reads a SELECT result into a bounded internal buffer. Count rows,
column/key metadata, collection slots, and cell
payload before growing that buffer. Apply snapshotted maxRows/maxResultBytes,
max_cell_bytes, the remaining runtime value budget, and cancellation/deadlines.
Client packet limits also bound individual driver buffers; logical result limits
are not a process heap cap.

A successful SELECT owns the connection's one active logical cursor, even though
its server result has been read. Close/exhaust it before preparing or executing
another statement or changing attributes. Closing another statement while it is
active raises DbStateError without destroying either handle. Exec and transaction
control also require closing/exhausting the cursor; metadata and inTransaction
inspection can run while it is active. CloseCursor discards
that buffer and keeps the prepared statement. CloseStatement can also release its
own cursor. Result-producing operations on different connections are independent.

Preflight rejection preserves existing state. Once execution is accepted, stale
results are cleared; a server statement error does not reuse old bindings and the
statement can execute again. Transport failure, deadline/cancellation, or an unsafe
result-limit failure breaks the session and invalidates all dependent handles.
There is no automatic retry. A DML operation can commit after the caller's local
timeout; an error is not proof that the server rolled it back. The live regression
observes this case explicitly and checks that the connector does not replay it.

The client's QueryResult destructor automatically drains unread protocol data.
On an unsafe result read/limit failure, send the error before that drain and
quarantine the worker rather than reusing its connection. Its retained worker slot
continues counting until actual cleanup finishes, including a delayed destructor.
Fetching consumes this buffer; fetch-one syntax does not imply streaming from the
server. Raw buffered data and decoded output can coexist during conversion, so
logical limits are not a process heap cap.

Fetch, fetchAll, and fetchColumn share one cursor and consume rows in SQL order.
Fetching before execute, after a failed execute, after explicit closeCursor, or
from a command without result columns raises DbStateError. Natural exhaustion
remains readable repeatedly: null for fetch, an empty list for fetchAll, and
DbEndOfResultsError for fetchColumn. SQL NULL is a valid scalar result. A negative
or out-of-range column index raises IndexError before consuming any row. Old
exhausted statements can be inspected without consuming another statement's cursor.

Associative maps preserve case-sensitive column labels in SQL order. Duplicate
labels or invalid UTF-8 labels raise DbTypeError; use SQL aliases to disambiguate
joins. FetchColumn uses the numeric position and does not require unique labels.

| MySQL value | Scripta value |
| --- | --- |
| SQL NULL | null |
| Integer, YEAR, BOOL/TINYINT | int; boolean column values are not guessed from 0/1 |
| Unsigned integer above signed 64-bit range | Exact decimal string |
| Finite FLOAT/DOUBLE | float |
| DECIMAL/NUMERIC | Exact string preserving returned scale |
| UTF-8 text, ENUM, SET | string; invalid UTF-8 raises DbTypeError |
| Binary strings/BLOB and BIT | bytes, preserving arbitrary byte values |
| JSON | UTF-8 string; decode explicitly with Json::decode |
| DATE | YYYY-MM-DD, including server zero dates |
| DATETIME/TIMESTAMP | YYYY-MM-DD HH:MM:SS with declared fractional precision (0–6 digits) |
| TIME | Signed HH:MM:SS with extended hours and declared fractional precision |
| Unsupported types (including GEOMETRY), malformed or non-finite values | DbTypeError |

Binary versus text uses MySQL's binary character-set metadata (63), rather than
whether a text collation compares bytes. A utf8mb4_bin text column remains a string.
Temporal conversion preserves server-returned fields without a client timezone
conversion; TIMESTAMP fields already reflect the server session timezone. UUID
text/binary columns follow their actual storage type. No implicit JSON, boolean,
date-object, decimal-float, or binary-text conversion occurs.

A decoding or decoded-output limit failure invalidates the result and discards
unread rows with bounded cleanup; the connection remains usable if cleanup succeeds.
Timeout/cancellation breaks the connection and invalidates all its handles. Already
returned values remain valid after cursor/connection closure. A failed fetchAll
never assigns a partial list. Fetch errors do not rewind rows or undo committed
writes; future drivers supporting write statements with RETURNING must preserve
this distinction too.

### Execution metadata and transactions

RowCount reads the latest successful DML execution, even after closeCursor.
Before execution, after an accepted failed execution, or after statement closure,
it raises DbStateError. For a successful SELECT it raises DbUnsupportedError;
use SELECT COUNT(*) and fetchColumn instead. Busy/type/parameter preflight errors
preserve existing execution metadata. Counts are ints; an affected count outside
signed 64-bit range raises DbLimitError rather than wrapping or converting to float.
A count-conversion failure does not undo an already completed command.

MySQL counts changed UPDATE rows by default; mysqlFoundRows=true requests matched
rows instead, for both execute and exec. Preserve native REPLACE and duplicate-key
update counts. Multi-row insert counts are totals; supported DDL through exec
returns zero (including DDL that also copies data).

LastInsertId captures nonzero generated-ID response metadata, rather than querying
a stale session LAST_INSERT_ID(). It returns the first server-generated ID for a
multi-row INSERT as an exact string, including values above signed int range.
Every accepted execute/exec clears the previous ID first; failures, SELECT, and
commands generating no ID leave null. Preflight errors preserve the previous ID.
Capture an ID immediately after insertion. Begin, commit, inspection and cleanup
do not clear a captured ID; a successful rollBack does. An ID is not proof that a
row committed. MySQL does not accept a non-null sequence argument.

Autocommit is explicitly on outside transactions; the connector sets session
completion_type=0 to prevent server defaults from chaining or releasing a session
on COMMIT/ROLLBACK. Begin rejects nesting, and commit/rollBack reject idle state.
Raw transaction/session-control SQL remains unavailable through prepare and exec.
Use InnoDB tables for transactional writes; nontransactional tables cannot provide
rollback guarantees. DDL may implicitly commit even if the DDL later fails; perform
schema changes before transactional writes. See the
[MySQL implicit-commit reference](https://dev.mysql.com/doc/refman/8.0/en/implicit-commit.html).

The connector enables and verifies MySQL's session transaction-state tracker before
publishing a connection. It reads public client session-state notifications and
uses an internal DO 0 probe to collect pending changes after errors or legacy EOF
packets. A deadlock rollback and a failed DDL implicit commit therefore update the
state reported by inTransaction. Failure to establish trustworthy state breaks the
connection. Servers without working transaction-state tracking are rejected during
connection setup; other server versions and MariaDB require separate validation.
See [MySQL session tracking](https://dev.mysql.com/doc/refman/8.0/en/session-state-tracking.html).

State refresh adds a server round trip after execution and around transaction
control; inTransaction also performs network I/O. It uses the same permissions,
deadlines, cancellation and worker quarantine as other network operations.
RowCount and lastInsertId are local metadata reads requiring owned open handles;
they do not perform I/O or require a new network grant.

A failed control acknowledgement cannot establish whether the transaction committed.
Report DbError with an unknown-outcome message and invalidate every dependent
handle. Local deadlines still raise DbTimeoutError; cancellation still raises
Interrupted. Neither proves rollback. There is no automatic retry, including after
a successful server COMMIT whose acknowledgement is lost. Explicit close attempts
ROLLBACK before disconnecting and invalidates handles even if cleanup fails. Dropped
connections close their sockets through Rust ownership; the server rolls back open
transactional work when it observes disconnection, which can happen after the
caller's timeout. Do not report an unconfirmed rollback as confirmed.

### Blocking I/O and worker lifecycle

Each MySQL connection has one dedicated worker owning the blocking client session.
The script polls responses at intervals of at most 10 ms and returns on cancellation
or the request deadline, including DNS/TCP/TLS/authentication wait for connection
startup and native execution/result reading for statement requests. Socket reads,
writes and TCP connects also have nonzero timeouts. Local CA-file access occurs
before the network deadline and follows normal filesystem cancellation checks.

There is a default maximum of 64 workers per driver descriptor. Timed-out or
cancelled connection attempts remain counted until their blocking client work
actually finishes. Dropping a handle signals worker shutdown without joining it
from a script destructor. A stalled DNS resolver or peer that keeps making partial
progress may delay worker teardown beyond the script deadline; threads cannot be
forcibly killed. The retained worker ceiling prevents unlimited abandoned tasks.
Do not interpret a returned deadline as proof that all OS activity has stopped.

Explicit close waits for worker acknowledgement within operationTimeoutMs. On
failure the shared layer still invalidates the handle and releases its connection
slot; an unfinished worker continues occupying its separate worker slot. The
client's normal disconnect releases server sessions. Interrupted SQL is quarantined
and its handles invalidated; interrupted transactional work is rolled back by
server disconnect. A disconnected or timed-out commit has an uncertain outcome.

The client uses its source-family bind path to work around mysql 26's first-address
failure behavior. DNS hostname identity stays unchanged for TLS verification.
Address-family fallback occurs only for failures before TCP establishment, not
for authentication failures, TLS rejection, or application SQL.

Backend errors are sanitized instead of displaying raw server messages or client
options. Safe server code and SQLSTATE are retained where available. Use environment
or host-supplied credentials and the runtime's existing secret-redaction settings
for source diagnostics. No connector debug output includes credentials or SQL.

## Ownership and limits

Every native operation verifies the concrete object type and registration/runtime
owner. Spoofed type names and foreign-runtime handles are rejected. A connection
owns backend resources, statements retain their connection, and sessions track
statements through weak references rather than creating ownership cycles.

Connection close invalidates all statements and releases its slot exactly once.
Statement cleanup failure disconnects the whole session. Unclosed statements whose
script handles are dropped remain backend-owned and count against the statement
ceiling until teardown; explicit closeStatement releases a slot promptly. A closed
old handle cannot remove a newer statement reusing the same backend identifier.
Host-retained handles can extend the session lifetime beyond runtime destruction.

Shared defaults are 64 connections per runtime, 256 backend statements per connection,
and 1 MiB SQL text per preparation. Failed connections consume no shared slot.
Connection metadata/credential payload is reported through NativeObject payload
accounting and rejected before worker allocation if it exceeds the logical value
budget. These limits are not a process heap cap. Parameter, cell, prepared-metadata and buffered-result accounting are applied to
execution. Decoded output is sized before growing its collections or copying cell
payloads, with separate cell and aggregate checks and cancellation checkpoints.

## Dependencies

The client is mysql 26.0.1 with default features disabled and minimal-rust plus
rustls-tls-ring. Optional derives, date/decimal conversions, binlog, and global
buffer pooling are disabled. MySQL and mysql_common use MIT/Apache-2.0 licensing.
Existing tempfile and standard-library dependencies are reused. A direct,
default-feature-free mysql_common dependency validates that the client cannot
rewrite the normalized placeholder SQL; it reuses the existing locked version. The MySQL client
is not part of the current CLI dependency graph.

The mysql 28.0.4 candidate resolved mysql_common 0.37.3, which failed actual Rust
1.85 compilation on unavailable integer APIs. The selected 26.x release preserves
the workspace minimum. Check upgrades on both the pinned compiler and Rust 1.85;
manifest metadata alone is insufficient evidence.

## Validation and fixtures

Shared deterministic backends test registration, strict signatures, ownership,
exact limits, failed destinations, cancellation, cleanup failures, backend identifier
reuse, and retained native resources. MySQL scanner tests exercise quotes, modes, markers, cancellation and LF/CRLF
comments. Shared tests verify execution dispatch and failure invalidation. MySQL
lifecycle unit tests exercise strict configuration,
policy denial, CA limits, sanitized malicious-server errors, refused TCP connections,
and controlled stalled/cancelled handshake fixtures.

Run focused tests on a native target with locked, cached dependencies:

```sh
cargo test -p scr-db -p scr-ext-mysql --locked --offline --target aarch64-apple-darwin
cargo +1.85.0 check -p scr-db -p scr-ext-mysql --locked --offline --target aarch64-apple-darwin
```

The real-server integration test is explicitly ignored in default runs. To run it,
install native mysqld/mysql (MySQL 8.x), OpenSSL and Python 3.9+, then invoke from
the workspace root on macOS ARM64:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 python3 crates/scr-ext-mysql/tests/support/mysql_fixture.py -- cargo test -p scr-ext-mysql --test mysql_lifecycle --test mysql_statements --test mysql_fetching --test mysql_transactions --locked --offline --target aarch64-apple-darwin -- --ignored --nocapture --test-threads=1
```

The helper creates its own temporary data directory, test users/database, and
CA/server certificates, binds only loopback, and shuts down/removes its server
when the command finishes. It does not modify an existing database installation.
It also starts a loopback TLS proxy using the disposable certificates. The dedicated
uncertain-commit test routes through it; it forwards COMMIT once and drops the real
server's successful acknowledgement. No production credentials or SQL are logged.
The proxy has a 1 MiB packet ceiling for its small fault-test workload.
The test verifies secure connection, CA/hostname rejection, authentication failure,
attributes, connection/CA boundaries, retained handles, and actual server session
cleanup. Statement tests additionally verify real bound data, hostile strings and
bytes, reuse after errors, preflight preservation, exact/default limits, cancellation,
and timed-out writes. Fetching tests cover mixed cursor consumption, empty and
exhausted results, strict type mapping, duplicate labels, recovery after decoding
errors, and exact/default decoded cell limits. Transaction tests cover affected counts, first/large IDs, found-row
settings, commit/rollback, successful/failed DDL commits, deadlock rollback,
connection/runtime cleanup, interrupted/timed-out writes, killed sessions and a
lost successful COMMIT acknowledgement. Decoder unit tests cover invalid
UTF-8, non-finite values, cancellation, and exact aggregate output sizes. Generated fixture credentials are public demo values, never production
credentials. A default run with this test ignored does not establish live-server
success. Use the equivalent native target and deployment settings on other hosts;
fixture portability and platform release support need native evidence.

Full workspace default/all-feature tests, Clippy and formatting follow the repository's
validation guidance. Debug validation on one host does not establish release support
or MySQL/MariaDB server compatibility on other systems. To exercise alternate SQL
quoting modes, prefix the fixture command with
`SCR_MYSQL_FIXTURE_SQL_MODE=ANSI_QUOTES,NO_BACKSLASH_ESCAPES,STRICT_TRANS_TABLES`.
