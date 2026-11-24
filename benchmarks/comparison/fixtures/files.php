<?php
$directory = $argv[1];
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
$target = $directory . '/output.txt';
file_put_contents($target, $text);
$actual = file_get_contents($target);
if ($actual !== $text) {
    throw new RuntimeException('file output mismatch');
}
echo $count, ':', strlen($actual), "\n";
