<?php
$texts = ["user-123@example.com", "invalid address", "a-9@host.org", "missing@"];
$total = 0;
for ($i = 0; $i < 20000; $i += 1) {
    if (preg_match("/^[a-z]+-[0-9]+@[a-z]+[.](com|org)$/", $texts[$i % 4]) === 1) { $total += 1; }
}
echo $total . "\n";
