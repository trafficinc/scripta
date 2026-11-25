# PostgreSQL connector

The optional `scr-ext-postgres` driver implements the shared [Db API](database-connectors.md)
using native PostgreSQL v3 preparation and binding. Enable the CLI `postgres`
feature; its DSN and discovery/package name are `pgsql`. SQLite stays enabled by
default. Installing a script library never installs a native driver.

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --locked --features postgres --target aarch64-apple-darwin
./target/aarch64-apple-darwin/debug/scripta version --verbose
```

Use `--features mysql,postgres` for both drivers, or
`--no-default-features --features postgres` for PostgreSQL alone. Use the native
Linux/Windows target on those hosts. These source builds do not change release
packages or certify additional platforms.

## Connections and host policy

```text
pgsql:host=localhost;port=5432;dbname=app
```

Host and dbname are required; port defaults to 5432. Fields are strict: no unknown
or duplicate keys, embedded credentials, quoted/escaped semicolons, socket paths,
or arbitrary startup options. Pass user and password separately. TCP, verified
certificate identity, UTF-8, and SCRAM-SHA-256 authentication are required. There
is no plaintext fallback, cleartext/MD5/trust authentication, or SCRAM channel
binding support in this version. Private CAs use `tlsCaFile`, subject to the host's
filesystem read permissions, roots and symlink policy. Hostname verification stays
on; public roots remain trusted alongside supplied CA certificates.

All common connection attributes and host ceilings apply. `mysqlFoundRows` is
unsupported. PostgreSQL reports matched UPDATE rows, including unchanged values.
Embedding hosts can compose both drivers in one registration:

```rust,ignore
scr_db::register(&mut runtime, vec![
    scr_ext_mysql::driver(),
    scr_ext_postgres::driver_with_options(scr_ext_postgres::PostgresOptions {
        destinations: Some(vec![scr_ext_postgres::Destination {
            host: "db.example.com".into(), port: 5432,
        }]),
        filesystem: shared_filesystem_policy,
        ..Default::default()
    })?,
])?;
```

The worker owns the verified TLS connection and protocol state. PostgreSQL's
`ReadyForQuery` I/T/E status determines idle, active and aborted transactions.
Cancellation/deadlines invalidate every dependent handle; blocking cleanup may
finish later, keeping its worker slot until it actually exits. Operations are
never replayed. Protocol frames are bounded before allocation; accumulated rows,
columns, cells and parameter payloads have separate logical limits. These are not
a process heap cap. SCRAM's server-requested iteration count is capped at 100,000.
The dependency supplies protocol encoding/decoding and SCRAM cryptography; the
extension supplies bounded transport and runtime integration, with no unsafe code.

## Binding and dialect

Use `?` with lists or unique `:name` markers with maps. The scanner translates
markers into `$1`, `$2`, etc.; values travel in native Bind messages. Native `$n`
input is rejected. Casts, quoted identifiers, standard and E strings, dollar
quotes, nested block comments, and LF/CRLF line comments preserve their contents.
Adjacent quoted string concatenation is rejected; use an explicit `||` expression.

PostgreSQL infers parameter types from SQL. Explicit casts make standalone values
unambiguous:

```scripta
$stmt = Db::prepare($db, "SELECT :name::text AS name, :amount::bigint AS amount");
Db::execute($stmt, ["name" => "Ada", "amount" => 2]);
$row = Db::fetch($stmt);
```

Null, bool, int, finite float and string parameters use length-delimited native
text input. Bytes use binary format and require an inferred bytea target or an
explicit `::bytea` cast; bytes never become text. Server conversion/validation
rules still apply. An untyped `SELECT ?` normally infers text, even for a Scripta
integer, so cast it to bigint when an integer result is needed.

A lone public `?` means a parameter. Write `??` for PostgreSQL's standalone JSON
existence operator; `?|`, `?&` and `@?` remain operators:

```scripta
$stmt = Db::prepare($db, "SELECT '{\"key\":1}'::jsonb ?? :key::text AS present");
Db::execute($stmt, ["key" => "key"]);
```

The first subset accepts ordinary SELECT/INSERT/UPDATE/DELETE, including DML
RETURNING, and parameterless table/index DDL through exec. WITH, COPY, CALL, DO,
raw session/transaction commands and stacked statements are rejected. SQL grammar
and schema identifiers remain backend-specific. Required session settings are
established internally. A reported change to standard_conforming_strings or
client_encoding breaks the connection rather than reusing incompatible lexical
or encoding state.

## Results and generated IDs

| PostgreSQL type | Scripta value |
| --- | --- |
| SQL NULL | null |
| boolean | bool |
| smallint/integer/bigint/oid | int |
| real/double precision | finite float; non-finite values fail |
| numeric | Exact string, including scale |
| text/char/varchar/name, JSON/JSONB, UUID | UTF-8 string |
| bytea | bytes, including NUL and invalid UTF-8 |
| date/time/timestamp/timestamptz/interval | Server text; ISO date and interval output, timezone UTC |
| Arrays, ranges, custom and other unsupported types | DbTypeError; cast explicitly to text if appropriate |

Native results request text format, avoiding float conversion of numeric and
implicit local-time conversions of temporal values. JSON text is not decoded to
a Scripta map. PostgreSQL controls JSONB normalization and fixed-char padding.
Associative labels preserve order/case and must be unique; scalar projection uses
column position. Exhaustion and SQL NULL/false remain distinct under the shared
fetch contract. One buffered logical cursor owns a connection at a time.

Prefer the ID returned by the actual write:

```scripta
$stmt = Db::prepare($db, "INSERT INTO users (name) VALUES (:name) RETURNING id");
Db::execute($stmt, ["name" => "Ada"]);
$id = Db::fetchColumn($stmt);
$count = Db::rowCount($stmt);
```

RETURNING rows must be naturally exhausted before rowCount is available. Explicitly
discarding unread RETURNING rows makes the count unavailable. SELECT rowCount is
unsupported; non-result DML returns its command count and exec DDL returns zero.

`Db::lastInsertId($db, "users_id_seq")` queries session-local `currval` using a
bound regclass name and returns an exact integer string. It requires an idle
cursor, network permission, and normal deadlines/cancellation. No sequence argument
raises DbUnsupportedError; undefined currval raises DbError. Qualified/quoted
sequence names follow PostgreSQL regclass rules. Neither lastval nor MAX(id) is
used. Triggers using other sequences do not change a named sequence's result.
Sequence allocations survive rollbacks and do not prove that a row exists or
committed. Inspecting insert IDs preserves error-history snapshots, as other
shared metadata inspection does.

## Transactions and errors

Begin rejects nesting; commit and rollBack reject idle state. A statement error
inside a PostgreSQL transaction generally aborts it. inTransaction remains true;
commit rejects the aborted state with DbStateError, and rollBack is required.
PostgreSQL DDL is transactional in the supported subset; it does not receive
MySQL's implicit-commit behavior. Explicit close confirms rollback of an active
transaction before disconnecting; Drop closes the socket through Rust ownership.

Lost COMMIT acknowledgement leaves an unknown outcome, raises a sanitized error,
and invalidates the connection and statements. No automatic retry occurs. A
caller timeout/cancellation does not prove rollback. Server errors expose validated
SQLSTATE in errorInfo, with a null driverCode because PostgreSQL has no analogous
numeric error code. Raw server messages/details, SQL and bound values are omitted.

A package can require the compiled operations without requiring a live server:

```toml
[extensions]
pgsql = { version = "^0.2", capabilities = ["prepare.native", "dml.returning", "lastInsertId.sequence", "transactions.state"] }
```

## Examples and native validation

[postgres.scripta](../examples/postgres.scripta) reads SCR_PG_DSN, SCR_PG_USER,
SCR_PG_PASSWORD and optional SCR_PG_CA_FILE. It runs a read-only parameterized
query and prints `Ada: 2`; it does not create schema or modify rows.

The isolated test fixture requires PostgreSQL initdb/pg_ctl/psql and openssl on
PATH. It creates only a temporary loopback SCRAM/TLS cluster and shuts it down;
it does not start a system service or connect to an existing database.

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 python3 crates/scr-ext-postgres/tests/support/postgres_fixture.py -- cargo test -p scr-ext-postgres --test postgres --locked --target aarch64-apple-darwin -- --ignored --test-threads=1
MACOSX_DEPLOYMENT_TARGET=15.0 python3 crates/scr-ext-postgres/tests/support/postgres_fixture.py -- cargo test -p scr-cli --all-features --test database_extensions postgres_public --locked --target aarch64-apple-darwin -- --ignored
MACOSX_DEPLOYMENT_TARGET=15.0 python3 crates/scr-ext-postgres/tests/support/postgres_fixture.py -- cargo test -p scr-cli --all-features --test database_extensions shared_driver_contract --locked --target aarch64-apple-darwin -- --ignored
```

The common contract fixture also runs unchanged under the MySQL fixture. Native
PostgreSQL tests cover typing, binding, cursors, limits, cancellation, TLS and
authentication failures, aborted transactions, DDL rollback, two-session IDs,
trigger sequences, and suppression of an actual successful COMMIT acknowledgement.
Normal cargo test runs deliberately leave live-server tests ignored; an ignored
test is not a passing native validation receipt. See the temporary review record
for the local checks performed; release-platform evidence remains separate.
