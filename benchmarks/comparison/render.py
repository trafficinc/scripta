#!/usr/bin/env python3
"""Generate comparison tables from complete, verified measurement evidence."""
import argparse
import json
import math
import os
from pathlib import Path
import statistics

from run import LANGUAGES, WORKLOADS

START = "<!-- BEGIN LANGUAGE BENCHMARKS -->"
END = "<!-- END LANGUAGE BENCHMARKS -->"


def render(report, methodology_link="benchmarks/comparison/README.md", table_only=False):
    if report.get("schema") != 1 or report.get("status") != "complete" or report.get("unavailable"):
        raise ValueError("only a complete comparison can supply the README table")
    samples = report["samples"]
    if not isinstance(samples, int) or samples < 1:
        raise ValueError("invalid sample count")
    results = {}
    for row in report["results"]:
        key = (row["workload"], row["language"])
        values = row["observations_ms"]
        if key in results or len(values) != samples or any(not math.isfinite(v) or v <= 0 for v in values):
            raise ValueError("duplicate result or invalid observations")
        if row["median_ms"] != statistics.median(values):
            raise ValueError("median differs from raw observations")
        results[key] = row
    if set(results) != {(name, language) for name, _, _, _ in WORKLOADS for language in LANGUAGES}:
        raise ValueError("comparison matrix is incomplete or contains unknown rows")
    lines = [
        "| Benchmark | Work size | PHP (ms) | Python (ms) | Scripta (ms) |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for name, label, size, _ in WORKLOADS:
        rows = [results[(name, language)] for language in LANGUAGES]
        if any(row["size"] != size or row["label"] != label for row in rows):
            raise ValueError("workload definitions differ from the measured suite")
        times = " | ".join(f"{row['median_ms']:.3f}" for row in rows)
        lines.append(f"| {label} | {size} | {times} |")
    if table_only:
        return "\n".join(lines) + "\n"
    lines = ["Median whole-process wall time in milliseconds; **lower is better**.", ""] + lines
    lines.extend([
        "",
        "Measured locally,",
        f"with {samples} samples after {report['warmups']} warmups per workload/runtime.",
        "Scripta uses an optimized native release build. PHP runs without php.ini,",
        "with CLI OPcache, PHP JIT, and PCRE JIT disabled; Python runs in isolated mode.",
        "Timings include startup, parsing, execution, and output validation work inside each script;",
        "filesystem caches are warm. These workloads compare their implementations and runtime APIs,",
        "including collection copying and regex compilation/caching; they are not an overall language ranking.",
        f"See the [methodology and reproduction commands]({methodology_link}).",
        "Raw samples and environment details are retained locally in ignored benchmark JSON files.", "",
    ])
    return "\n".join(lines)


def replace_section(data, section):
    # Preserve the existing README's newline convention, including CRLF input.
    newline = b"\r\n" if b"\r\n" in data else b"\n"
    start, end = START.encode(), END.encode()
    if data.count(start) != 1 or data.count(end) != 1 or data.index(start) >= data.index(end):
        raise ValueError("README must contain one ordered pair of benchmark markers")
    first = data.index(start) + len(start)
    last = data.index(end)
    body = section.rstrip("\n").encode().replace(b"\n", newline)
    return data[:first] + newline + body + newline + data[last:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("--readme", type=Path)
    parser.add_argument("--performance", type=Path, help="update the table-only performance reference")
    args = parser.parse_args()
    report = json.loads(args.evidence.read_text())
    updates = []
    for path, table_only in ((args.readme, False), (args.performance, True)):
        if path is None:
            continue
        parent = path.resolve().parent
        methodology = Path(os.path.relpath(Path(__file__).resolve().parent / "README.md", parent)).as_posix()
        section = render(report, methodology, table_only=table_only)
        updates.append((path, replace_section(path.read_bytes(), section)))
    # Validate all destinations before writing either document.
    for path, data in updates:
        path.write_bytes(data)
    if not updates:
        print(render(report), end="")


if __name__ == "__main__":
    main()
