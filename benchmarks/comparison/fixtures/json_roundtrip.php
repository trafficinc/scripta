<?php
$data = ["name" => "sample", "values" => [1, 2, 3, 4], "meta" => ["ok" => true, "count" => 7]];
$total = 0;
for ($i = 0; $i < 20000; $i += 1) {
    $decoded = json_decode(json_encode($data, JSON_THROW_ON_ERROR), true, 512, JSON_THROW_ON_ERROR);
    if ($decoded !== $data) { throw new Exception("roundtrip mismatch"); }
    $total += $decoded["meta"]["count"] + $decoded["values"][3];
}
echo $total . "\n";
