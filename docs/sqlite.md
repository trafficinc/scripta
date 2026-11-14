# SQLite extension

SQLite is an optional built-in extension. Enable it when building the CLI:

```sh
cargo build -p scr-cli --locked --features sqlite
cargo run -p scr-cli --locked --features sqlite -- examples/sqlite.scripta
```

The extension is disabled in default builds. `scripta version --verbose` reports
`extensions: sqlite` or `extensions: none`. It registers four native functions;
script packages and `require` do not enable it. The
[runnable example](../examples/sqlite.scripta) uses an in-memory database and prints:

```text
1: Ada
2: 世界
```

The `scr-ext-sqlite` crate uses [rusqlite](https://github.com/rusqlite/rusqlite)
with bundled SQLite. Building it requires a C compiler as well as Rust; the
finished binary does not require an installed SQLite library or the `sqlite3`
command. This version uses rusqlite 0.40 to retain the project's Rust 1.85 minimum.
Check platform-specific native toolchains before producing a release.

## Script API

| Function | Behavior |
| --- | --- |
| `Sqlite::open(path: string, readOnly: bool optional, busyTimeoutMs: int optional): SqliteDatabase` | Opens a file database, creating it if needed in writable mode, or opens a private `:memory:` database. Defaults: `readOnly = false`, `busyTimeoutMs = 1000`. The busy timeout must be between 0 and 5000 ms. |
| `Sqlite::execute(db: SqliteDatabase, sql: string, params: list optional): int` | Executes one statement that returns no columns. Returns the directly affected row count for DML, excluding trigger changes; DDL and transaction statements return zero. Parameters default to `[]`. |
| `Sqlite::query(db: SqliteDatabase, sql: string, params: list optional): list` | Executes one read-only statement returning columns. Returns a list of maps keyed by column name, including `[]` for no rows. Parameters default to `[]`. |
| `Sqlite::close(db: SqliteDatabase): null` | Closes the connection and releases locks. Repeated closes succeed. Subsequent execute/query calls fail. |

Database objects have read-only `closed: bool` and `readOnly: bool` properties.
Handles belong to the runtime registration that opened them. A host cannot move
a handle into another runtime and use or close it through that runtime's SQLite
functions. Dropping the last reference to a connection releases it; outstanding
transactions roll back. Host-held references keep a connection alive after the
runtime is dropped.

```scr
<?scr
$db = Sqlite::open("app.db");
try {
    Sqlite::execute($db, "CREATE TABLE IF NOT EXISTS users (name TEXT NOT NULL)");
    $changed = Sqlite::execute($db, "INSERT INTO users (name) VALUES (?)", ["Ada"]);
    $rows = Sqlite::query($db, "SELECT name FROM users ORDER BY rowid");
    foreach ($rows as $row) {
        echo $row["name"] . "\n";
    }
} finally {
    Sqlite::close($db);
}
```

Each call accepts exactly one SQL statement. Trailing whitespace, comments, and
semicolons are allowed; multiple statements are rejected before execution.
`execute` rejects statements returning columns, including `SELECT` and
`INSERT ... RETURNING`. `query` rejects mutating statements, including writes
with `RETURNING`; this prevents result-limit errors from hiding mutations.
Use separate calls for schema changes, inserts, and queries.

## Parameters and values

Use `?` or `?NNN` placeholders with a positional parameter list. Values are
bound through SQLite, never concatenated into SQL. Parameter counts must match
SQLite's placeholder indices; for example, `?3` requires a three-element list.
Named placeholders such as `:name` and parameter maps are unsupported. Bind
values, not table or column identifiers; choose identifiers from trusted code.

| Scripta parameter | SQLite value |
| --- | --- |
| `null` | NULL |
| `bool` | INTEGER 0 or 1 |
| `int` | Signed 64-bit INTEGER |
| Finite `float` | REAL |
| `string` | UTF-8 TEXT, including embedded NUL bytes |

Lists, maps, callables, native objects, and non-finite floats cannot be bound.
SQLite NULL, INTEGER, REAL, and TEXT values return as Scripta null, int, float,
and string respectively. SQLite's storage affinity can change a stored value's
type; returned integers are not automatically converted to bools.

BLOB results are not supported by this extension, despite the core `bytes` type,
and raise `SqliteTypeError` when returned. Use SQL `hex(column)` for text output.
Invalid UTF-8 TEXT and non-finite
REAL results also raise `SqliteTypeError`. Duplicate column names are rejected;
use aliases in joins, such as `SELECT a.id AS a_id, b.id AS b_id ...`. Result maps
preserve column order and use case-sensitive string keys.

## Transactions and errors

Use SQL transaction statements through `execute`:

```scr
<?scr
$db = Sqlite::open(":memory:");
try {
    Sqlite::execute($db, "CREATE TABLE t (id INTEGER PRIMARY KEY)");
    Sqlite::execute($db, "BEGIN");
    try {
        Sqlite::execute($db, "INSERT INTO t VALUES (?)", [1]);
        Sqlite::execute($db, "COMMIT");
    } catch ($error) {
        Sqlite::execute($db, "ROLLBACK");
        throw $error;
    }
} finally {
    Sqlite::close($db);
}
```

`BEGIN`, `COMMIT`, `ROLLBACK`, and savepoints use SQLite's ordinary semantics.
SQLite may automatically roll back a transaction after some serious statement
errors; an additional `ROLLBACK` can then fail. Structure application recovery
around the statements and errors it expects. There is no implicit multi-call
transaction or automatic retry. Closing/dropping an uncommitted connection
rolls back its transaction. Cancellation and fatal runtime limits bypass script
`finally`; Rust cleanup releases connections once their owners are dropped.

| Error | Meaning |
| --- | --- |
| `TypeError` | Wrong native arguments, unsupported parameter types, or a foreign/fake database handle. |
| `RangeError` | Busy timeout outside 0–5000 ms. |
| `PermissionError` | Missing filesystem capability, a path outside SQLite roots, a write through a read-only connection, or SQL denied by the connection policy. |
| `SqliteConstraintError` | Uniqueness, NOT NULL, CHECK, or foreign-key constraint failure. Foreign keys are enabled on every connection. |
| `SqliteBusyError` | Database/table locks could not be acquired within the busy timeout. |
| `SqliteLimitError` | Result, parameter, connection, SQL/value size, execution-step, or statement deadline limit exceeded. |
| `SqliteTypeError` | A result contains BLOB, invalid UTF-8 TEXT, or non-finite REAL data. |
| `SqliteError` | Other SQLite or API errors, including malformed SQL, missing tables, a closed handle, and unsupported statement shapes. |
| `IoError` | Host path resolution failed before SQLite opened the database. |
| `Interrupted` | Runtime cancellation; reserved and not catchable by scripts. |

Ordinary errors are catchable and retain the script call location. No partial
query results are returned after an error. A connection can usually be reused
after an error; transaction state follows SQLite's error behavior.

## Permissions and SQL policy

File database paths resolve against the runtime's working directory, including
changes made by `Dir::change`, rather than the including script's directory.
Parent directories must already exist. Empty paths, NUL-containing paths,
`file:` URIs, and remote URLs are rejected. `:memory:` is a private connection;
there are no shared-memory URI databases.

Opening a writable file requires `filesystem.read` and `filesystem.write`.
Opening a read-only file and operating on file connections require
`filesystem.read`; mutating SQL also requires `filesystem.write`. Read-only file
connections use SQLite read-only open flags. `:memory:` needs no filesystem
capability, but its optional read-only setting still prevents mutating SQL.
`close` performs no filesystem capability check.

Connection policy denies `ATTACH`, `DETACH`, all PRAGMA statements, virtual-table
creation/deletion, and extension loading. `VACUUM`/`VACUUM INTO` require an
internal attachment and are also denied. Use `SELECT` against `sqlite_schema`
for schema inspection. These restrictions apply during statement preparation,
including statements reached through triggers/views. Temporary data uses memory.
There is no loadable extension API, custom SQL function API, or prepared-statement
handle API in this version.

For embedders, SQLite root restrictions are configured separately from the
standard library's `FilesystemPolicy`:

```rust
use scr_ext_sqlite::{SqliteOptions, register_with_options};
use scr_runtime::Runtime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new();
    let data = std::fs::canonicalize("data")?;
    register_with_options(&mut runtime, SqliteOptions {
        roots: Some(vec![data]),
        ..Default::default()
    })?;
    Ok(())
}
```

Roots must be existing absolute directories; `Some(vec![])` denies all file
databases while allowing private memory databases. Existing paths and parents
are canonicalized before checking roots; SQLite is opened with its no-follow
flag to reject a final symlink introduced after that check. As with other
filesystem APIs, concurrent replacement of ancestor directories is not isolated
by these checks. SQLite may create journal/WAL sidecar files beside the database.
The default CLI registers unrestricted roots and uses runtime filesystem
capabilities. Embedders with restricted stdlib roots must also configure SQLite
roots; stdlib policy does not automatically restrict this separate crate.

## Limits and cancellation

`SqliteOptions` lets hosts configure positive `max_connections` (64), `max_rows`
(10,000), and `max_result_bytes` (8 MiB), plus `max_steps` (5,000,000; minimum
1000) and `statement_timeout` (5 seconds). Rows beyond the limit raise an error
rather than silently truncating. The byte limit includes conservative row,
cell, column-key, and string overhead; it also bounds copied parameter data.
The runtime's `max_value_bytes` further reduces that byte budget.

Fixed connection limits include 1 MiB of SQL, 1 MiB for SQLite values/encoded
rows, 1024 parameter indices, 256 columns, expression depth 100, and trigger
depth 32. Limits are per connection or native call; they do not impose a hard
process RSS limit, database-file quota, or total script SQL-work budget.

SQLite's progress callback checks cancellation, a per-call step budget, and a
statement deadline approximately every 1000 virtual-machine operations,
including long-running recursive queries. These limits supplement the runtime's
script instruction limit, which does not count SQLite operations. Lock waits
are bounded by `busyTimeoutMs`; the progress callback does not run during those
waits. Some OS I/O and native work also occur between callbacks, so the deadline
is cooperative rather than a hard wall-clock guarantee. Hooks are removed after
every call, including failed calls.

## Validation and release builds

```sh
cargo test -p scr-ext-sqlite --locked
cargo test -p scr-cli --locked --features sqlite --test sqlite_extension
cargo test -p scr-cli --locked --no-default-features --test sqlite_extension
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test --workspace --no-default-features --locked
```

Default CLI builds do not link this extension. A full workspace build/test still
builds `scr-ext-sqlite` as a workspace member and therefore needs a C compiler.
Use `-p scr-cli --no-default-features` to build only the CLI without SQLite.

For a native release, build with `--features sqlite` and tell the release
packager to use the same feature graph. For example, on macOS ARM64:

```sh
cargo build -p scr-cli --profile dist --target aarch64-apple-darwin --locked --features sqlite
cargo run -p scr-release --locked -- package --binary target/aarch64-apple-darwin/dist/scripta --target aarch64-apple-darwin --out-dir dist/sqlite --features sqlite
cargo run -p scr-release --locked -- verify --archive dist/sqlite/scripta-0.1.0-aarch64-apple-darwin.tar.gz
cargo run -p scr-release --locked -- smoke --archive dist/sqlite/scripta-0.1.0-aarch64-apple-darwin.tar.gz
```

The archive records its declared extensions and includes the selected dependency
notices. Smoke validates the executable's extension list against that metadata
and runs the SQLite example for enabled archives. Use separate output directories
for different feature variants, since their archive filenames are the same.
These commands do not publish a release or establish support for untested targets.
