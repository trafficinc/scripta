<?php
$values = range(0, 1999);
$total = 0;
$count = 0;
for ($batch = 0; $batch < 200; $batch += 1) {
    $mapped = array_map(fn($x) => $x * 3, $values);
    $filtered = array_values(array_filter($mapped, fn($x) => $x % 2 == 0));
    $count += count($filtered);
    foreach ($filtered as $x) { $total += $x; }
}
echo $count . ":" . $total . "\n";
