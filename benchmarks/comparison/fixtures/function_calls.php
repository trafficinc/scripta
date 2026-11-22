<?php
function increment(int $value): int { return $value + 1; }
$total = 0;
for ($i = 0; $i < 200000; $i += 1) { $total += increment($i); }
echo $total . "\n";
