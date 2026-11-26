<?php
declare(strict_types=1);

$fixture = json_decode(file_get_contents($argv[1]), true, 512, JSON_THROW_ON_ERROR);
$results = [];
function square(int $value): int { return $value * $value; }
function even(int $value): bool { return $value % 2 === 0; }
function add(int $acc, int $value): int { return $acc + $value; }
function rankValue(array $value): int { return $value['rank']; }
function compareRank(array $left, array $right): int { return $left['rank'] <=> $right['rank']; }
function makeOffset(int $amount): Closure { return static fn(int $value): int => $value + $amount; }
function appendValue(array $acc, int $value): array { $acc[] = $value; return $acc; }
function canonical(mixed $value): mixed {
    if (!is_array($value)) return $value;
    if (array_is_list($value)) return array_map('canonical', $value);
    ksort($value, SORT_STRING);
    $out = [];
    foreach ($value as $key => $item) $out[] = [$key, canonical($item)];
    return $out;
}
foreach ($fixture['datasets'] as $dataset) {
    $values = $dataset['values'];
    $rows = $dataset['rows'];
    // Preserve numeric string keys as strings: PHP associative arrays coerce them.
    $entries = $dataset['entries'];
    $captured = makeOffset(7);
    $mapMap = static function (callable $callback) use ($entries): array {
        $out = [];
        foreach ($entries as [$key, $value]) $out[] = [$key, $callback($value, $key)];
        return $out;
    };
    $mapFilter = static function (callable $callback) use ($entries): array {
        $out = [];
        foreach ($entries as [$key, $value]) if ($callback($value, $key)) $out[] = [$key, $value];
        return $out;
    };
    $mapReduce = static function (callable $callback, mixed $initial) use ($entries): mixed {
        $acc = $initial;
        foreach ($entries as [$key, $value]) $acc = $callback($acc, $value, $key);
        return $acc;
    };
    $sortKey = static function () use ($rows): array {
        $decorated = array_map(static fn(array $row): array => [rankValue($row), $row], $rows);
        usort($decorated, static fn(array $left, array $right): int => $left[0] <=> $right[0]);
        return array_map(static fn(array $item): array => $item[1], $decorated);
    };
    $sortCompare = static function () use ($rows): array {
        $out = $rows;
        usort($out, 'compareRank');
        return $out;
    };
    $cases = [
        ['list_map', static fn(): array => array_map('square', $values)],
        ['list_map_capture', static fn(): array => array_map($captured, $values)],
        ['list_filter', static fn(): array => array_values(array_filter($values, 'even'))],
        ['list_reduce', static fn(): int => array_reduce($values, 'add', 0)],
        ['map_map', static fn(): array => $mapMap(static fn(int $v, mixed $k): int => $v + 1)],
        ['map_filter', static fn(): array => $mapFilter(static fn(int $v, mixed $k): bool => even($v))],
        ['map_reduce', static fn(): int => $mapReduce(static fn(int $a, int $v, mixed $k): int => $a + $v, 0)],
        ['record_map', static fn(): array => array_map('rankValue', $rows)],
        ['record_sort_key', $sortKey],
        ['record_sort_compare', $sortCompare],
        ['reduce_growing_copy', static fn(): array => array_reduce($values, 'appendValue', [])],
    ];
    $samples = array_fill(0, count($cases), []);
    $repeats = [];
    foreach ($cases as [$name, $run]) {
        $start = hrtime(true);
        $count = 0;
        do { $sink = $run(); $count++; } while (hrtime(true) - $start < 100000000);
        $perCall = (hrtime(true) - $start) / $count;
        $repeats[] = max(1, min(10000, (int)ceil(10000000 / $perCall)));
    }
    for ($sample = 0; $sample < 11; $sample++) {
        for ($position = 0; $position < count($cases); $position++) {
            $i = $sample % 2 === 0 ? $position : count($cases) - 1 - $position;
            [$name, $run] = $cases[$i];
            $start = hrtime(true);
            for ($j = 0; $j < $repeats[$i]; $j++) $sink = $run();
            $elapsed = (hrtime(true) - $start) / 1000000 / $repeats[$i];
            $encoded = str_starts_with($name, 'record_sort_') ? canonical($sink) : $sink;
            if ($encoded !== $dataset['expected'][$name]) throw new RuntimeException("incorrect output: $name");
            $samples[$i][] = $elapsed;
        }
    }
    foreach ($cases as $i => [$name, $run]) {
        $results[] = ['operation' => $name, 'size' => $dataset['size'], 'batch' => $repeats[$i], 'samples_ms' => $samples[$i]];
    }
}
echo json_encode(['runtime' => 'php', 'version' => PHP_VERSION, 'binary' => PHP_BINARY,
    'arch' => php_uname('m'), 'int_bytes' => PHP_INT_SIZE,
    'settings' => ['opcache.enable_cli' => ini_get('opcache.enable_cli'), 'opcache.jit' => ini_get('opcache.jit'),
                   'memory_limit' => ini_get('memory_limit')], 'results' => $results], JSON_THROW_ON_ERROR), "\n";
