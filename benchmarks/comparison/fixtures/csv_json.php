<?php
$directory = $argv[1];
$source = fopen($directory . '/input.csv', 'rb');
$rows = [];
try {
    while (($row = fgetcsv($source, 0, ',', '"', '')) !== false) {
        $rows[] = $row;
    }
} finally {
    fclose($source);
}
$values = [];
foreach ($rows as $row) {
    $values[] = ['id' => (int)$row[0], 'name' => $row[1], 'value' => (int)$row[2] * 3];
}
$target = $directory . '/output.json';
file_put_contents($target, json_encode($values, JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR));
$decoded = json_decode(file_get_contents($target), true, 512, JSON_THROW_ON_ERROR);
if ($decoded !== $values) {
    throw new RuntimeException('JSON output mismatch');
}
$total = 0;
foreach ($decoded as $row) {
    $total += $row['id'] + $row['value'];
}
echo count($decoded), ':', $total, "\n";
