"""Eager callback workloads; invoked in isolated mode by run.py."""
import functools
import json
import platform
import sys
import time

with open(sys.argv[1], encoding="utf-8") as source:
    fixture = json.load(source)
results = []
sink = None


def canonical(value):
    if isinstance(value, dict):
        return [[key, canonical(value[key])] for key in sorted(value)]
    if isinstance(value, list):
        return [canonical(item) for item in value]
    return value


def square(value):
    return value * value


def even(value):
    return value % 2 == 0


def add(acc, value):
    return acc + value


def rank(value):
    return value["rank"]


def compare_rank(left, right):
    return (left["rank"] > right["rank"]) - (left["rank"] < right["rank"])


def make_offset(amount):
    return lambda value: value + amount


def append_mutating(acc, value):
    acc.append(value)
    return acc


for dataset in fixture["datasets"]:
    values, rows, entries = dataset["values"], dataset["rows"], dataset["entries"]
    captured = make_offset(7)

    def map_map(callback):
        return [[key, callback(value, key)] for key, value in entries]

    def map_filter(callback):
        return [[key, value] for key, value in entries if callback(value, key)]

    def map_reduce(callback, initial):
        acc = initial
        for key, value in entries:
            acc = callback(acc, value, key)
        return acc

    cases = [
        ("list_map", lambda: list(map(square, values))),
        ("list_map_capture", lambda: list(map(captured, values))),
        ("list_filter", lambda: list(filter(even, values))),
        ("list_reduce", lambda: functools.reduce(add, values, 0)),
        ("map_map", lambda: map_map(lambda value, key: value + 1)),
        ("map_filter", lambda: map_filter(lambda value, key: even(value))),
        ("map_reduce", lambda: map_reduce(lambda acc, value, key: acc + value, 0)),
        ("record_map", lambda: list(map(rank, rows))),
        ("record_sort_key", lambda: sorted(rows, key=rank)),
        ("record_sort_compare", lambda: sorted(rows, key=functools.cmp_to_key(compare_rank))),
        ("reduce_growing_copy", lambda: functools.reduce(lambda acc, value: acc + [value], values, [])),
        ("reduce_growing_mutate_python_only", lambda: functools.reduce(append_mutating, values, [])),
    ]
    samples = [[] for _ in cases]
    repeats = []
    for _, run in cases:
        start = time.perf_counter_ns()
        count = 0
        while count == 0 or time.perf_counter_ns() - start < 100_000_000:
            sink = run()
            count += 1
        per_call_ns = (time.perf_counter_ns() - start) / count
        repeats.append(max(1, min(10000, int(-(-10_000_000 // per_call_ns)))))
    for sample in range(11):
        order = range(len(cases)) if sample % 2 == 0 else reversed(range(len(cases)))
        for index in order:
            name, run = cases[index]
            start = time.perf_counter_ns()
            for _ in range(repeats[index]):
                sink = run()
            elapsed = (time.perf_counter_ns() - start) / 1e6 / repeats[index]
            expected_name = "reduce_growing_copy" if name.endswith("_python_only") else name
            # Map adapters already emit typed entries; only records need encoding.
            encoded = canonical(sink) if name.startswith("record_sort_") else sink
            if encoded != dataset["expected"][expected_name]:
                raise RuntimeError(f"incorrect output: {name}")
            samples[index].append(elapsed)
    for index, (name, _) in enumerate(cases):
        results.append(dict(operation=name, size=dataset["size"], batch=repeats[index], samples_ms=samples[index]))
print(json.dumps(dict(runtime="python", version=sys.version, arch=platform.machine(),
                     implementation=platform.python_implementation(), flags=str(sys.flags), results=results)))
