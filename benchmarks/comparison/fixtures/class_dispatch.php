<?php
class Counter {
    public int $value = 0;
    function next(): int {
        $this->value += 1;
        return $this->value;
    }
}
$counter = new Counter();
$total = 0;
for ($i = 0; $i < 200000; $i += 1) { $total += $counter->next(); }
echo $counter->value . ":" . $total . "\n";
