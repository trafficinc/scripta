<?php
$total = 0;
for ($i = 0; $i < 1000000; $i += 1) {
    $total += ($i % 97) * 3;
}
echo $total . "\n";
