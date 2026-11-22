<?php
function mix(int $value): int {
    return ($value % 97) * 3 + 1;
}
function work(string $case, int $n, string $directory, string $url, string $python): int {
    if ($case === 'numeric_float') {
        $total = 0.0;
        for ($i = 0; $i < $n; $i += 1) {
            $total += (float)($i % 97) * 0.5;
        }
        return (int)($total * 2.0);
    }
    if ($case === 'numeric_nested') {
        $total = 0;
        $width = max(1, intdiv($n, 20));
        for ($outer = 0; $outer < 20; $outer += 1) {
            for ($i = 0; $i < $width; $i += 1) {
                $total += ($outer + $i) % 97;
            }
        }
        return $total;
    }
    if ($case === 'numeric_bounds') {
        $total = 0;
        $bound = $n;
        for ($i = 0; $i < $bound; $i += 1) {
            if ($i % 11 === 0) {
                $bound -= 1;
            }
            $total += $i;
        }
        return $total;
    }
    if ($case === 'numeric_calls') {
        $total = 0;
        for ($i = 0; $i < $n; $i += 1) {
            $total += mix($i);
        }
        return $total;
    }
    if ($case === 'files') {
        $source = fopen($directory . '/input.txt', 'rb');
        $text = '';
        $count = 0;
        try {
            while (($line = fgets($source)) !== false) {
                $text .= strtoupper(rtrim($line, "\r\n")) . "\n";
                $count += 1;
            }
        } finally {
            fclose($source);
        }
        $target = $directory . '/output-php.txt';
        file_put_contents($target, $text);
        return $count * 100000 + strlen(file_get_contents($target));
    }
    if ($case === 'csv_json') {
        $source = fopen($directory . '/input.csv', 'rb');
        $rows = [];
        try {
            while (($row = fgetcsv($source, 0, ',', '"', '')) !== false) {
                $rows[] = $row;
            }
        } finally {
            fclose($source);
        }
        if ($rows[0][1] !== "comma, quote\" and\nline é") {
            throw new RuntimeException('CSV string mismatch');
        }
        $values = [];
        foreach ($rows as $row) {
            $values[] = ['id' => (int)$row[0], 'name' => $row[1], 'value' => (int)$row[2] * 3];
        }
        $decoded = json_decode(json_encode($values, JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR), true, 512, JSON_THROW_ON_ERROR);
        if ($decoded !== $values) {
            throw new RuntimeException('JSON roundtrip mismatch');
        }
        $total = 0;
        foreach ($decoded as $row) {
            $total += $row['id'] + $row['value'];
        }
        return $total;
    }
    if ($case === 'subprocess') {
        $total = 0;
        for ($i = 0; $i < 3; $i += 1) {
            $child = proc_open([$python, '-I', $directory . '/child.py'],
                [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes);
            if (!is_resource($child)) {
                throw new RuntimeException('child failed to start');
            }
            fclose($pipes[0]);
            $stdout = stream_get_contents($pipes[1]);
            $stderr = stream_get_contents($pipes[2]);
            fclose($pipes[1]);
            fclose($pipes[2]);
            if (proc_close($child) !== 0 || $stdout !== "child-ok\n" || $stderr !== '') {
                throw new RuntimeException('child output mismatch');
            }
            $total += strlen($stdout);
        }
        return $total;
    }
    if ($case === 'http_stream') {
        $expected = file_get_contents($directory . '/payload.bin');
        $context = stream_context_create(['http' => ['timeout' => 5, 'follow_location' => 0]]);
        $stream = fopen($url, 'rb', false, $context);
        $offset = 0;
        try {
            $headers = stream_get_meta_data($stream)['wrapper_data'];
            if (!str_contains($headers[0], ' 200 ')) {
                throw new RuntimeException('HTTP status mismatch');
            }
            while (!feof($stream)) {
                $chunk = fread($stream, 4096);
                if ($chunk === false || $chunk !== substr($expected, $offset, strlen($chunk))) {
                    throw new RuntimeException('HTTP bytes mismatch');
                }
                $offset += strlen($chunk);
            }
        } finally {
            fclose($stream);
        }
        return $offset;
    }
    if ($case === 'sqlite') {
        $db = new SQLite3(':memory:');
        $db->enableExceptions(true);
        try {
            $db->exec('CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)');
            $db->exec('BEGIN');
            for ($i = 0; $i < 256; $i += 1) {
                $statement = $db->prepare('INSERT INTO items VALUES (?, ?)');
                $statement->bindValue(1, $i, SQLITE3_INTEGER);
                $statement->bindValue(2, $i % 17, SQLITE3_INTEGER);
                $statement->execute()->finalize();
                $statement->close();
            }
            $db->exec('COMMIT');
            $row = $db->querySingle('SELECT COUNT(*) AS count, SUM(value) AS total FROM items', true);
            return $row['count'] * 10000 + $row['total'];
        } finally {
            $db->close();
        }
    }
    throw new RuntimeException('unknown workload');
}
[$case, $n, $batches, $directory, $url, $python] = array_slice($argv, 1);
$times = [];
$checksums = [];
for ($batch = 0; $batch < (int)$batches; $batch += 1) {
    $started = hrtime(true);
    $checksum = work($case, (int)$n, $directory, $url, $python);
    $times[] = (hrtime(true) - $started) / 1000000.0;
    $checksums[] = $checksum;
}
echo json_encode(['case' => $case, 'elapsed_ms' => $times, 'checksums' => $checksums], JSON_THROW_ON_ERROR), "\n";
