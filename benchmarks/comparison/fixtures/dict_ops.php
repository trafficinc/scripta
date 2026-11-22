<?php
$items = [];
for ($i = 0; $i < 256; $i += 1) { $items["key" . strval($i)] = $i; }
$total = 0;
for ($i = 0; $i < 40960; $i += 1) {
    $key = "key" . strval($i % 256);
    $items[$key] += 1;
    $total += $items[$key];
}
for ($i = 0; $i < 128; $i += 1) {
    $updated = $items;
    unset($updated["key" . strval($i)]);
    $items = $updated;
}
echo count($items) . ":" . $total . ":" . array_sum($items) . "\n";
