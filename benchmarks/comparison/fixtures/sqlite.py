import sqlite3

db = sqlite3.connect(":memory:", cached_statements=0, isolation_level=None)
try:
    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)")
    db.execute("BEGIN")
    for i in range(10000):
        db.execute("INSERT INTO items VALUES (?, ?)", (i, i % 17))
    db.execute("COMMIT")
    row = db.execute("SELECT COUNT(*) AS count, SUM(id) AS ids, SUM(value) AS total, SUM(id * value) AS weighted FROM items").fetchone()
    print(":".join(str(value) for value in row))
finally:
    db.close()
