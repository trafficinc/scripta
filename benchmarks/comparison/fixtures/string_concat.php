<?php
$text = "";
for ($i = 0; $i < 100000; $i += 1) { $text = $text . "abcdefgh"; }
echo strlen($text) . ":" . (int)(str_replace("abcdefgh", "", $text) === "") . "\n";
