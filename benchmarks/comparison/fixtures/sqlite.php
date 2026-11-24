<?php
$db = new SQLite3(':memory:');
$db->enableExceptions(true);
try {
    $db->exec('CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)');
    $db->exec('BEGIN');
    for ($i = 0; $i < 10000; $i += 1) {
        $statement = $db->prepare('INSERT INTO items VALUES (?, ?)');
        $statement->bindValue(1, $i, SQLITE3_INTEGER);
        $statement->bindValue(2, $i % 17, SQLITE3_INTEGER);
        $statement->execute()->finalize();
        $statement->close();
    }
    $db->exec('COMMIT');
    $row = $db->querySingle('SELECT COUNT(*) AS count, SUM(id) AS ids, SUM(value) AS total, SUM(id * value) AS weighted FROM items', true);
    echo $row['count'], ':', $row['ids'], ':', $row['total'], ':', $row['weighted'], "\n";
} finally {
    $db->close();
}
