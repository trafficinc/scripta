#!/usr/bin/env python3
"""Verified, randomized CLI comparison using only the Python standard library."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
LANGUAGES = ("php", "python", "scripta")
# name, table label, fixed work, independent expected stdout
WORKLOADS = (
    ("numeric_loop", "Numeric loop", "1,000,000 iterations", "143997165\n"),
    ("recursive_fib", "Recursive Fibonacci", "fib(28), no memoization", "317811\n"),
    ("list_pipeline", "List pipeline", "2,000 items × 200 passes", "200000:599400000\n"),
    ("string_concat", "String concatenation", "100,000 × 8-byte appends", "800000:1\n"),
    ("dict_ops", "Dictionary operations", "256 keys; 40,960 updates; 128 copy-removals", "128:8519680:44992\n"),
    ("class_dispatch", "Class dispatch", "200,000 method calls", "200000:20000100000\n"),
    ("regex_match", "Regex match", "20,000 matches attempted", "10000\n"),
    ("json_roundtrip", "JSON roundtrip", "20,000 encode/decode pairs", "220000\n"),
    ("function_calls", "Function calls", "200,000 calls", "20000100000\n"),
    ("startup", "Startup", "Empty script", ""),
)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def environment():
    # Eliminate inherited Scripta budgets/permissions as a source of hidden variation.
    env = {key: value for key, value in os.environ.items() if not key.startswith("SCR_")}
    env.update(LC_ALL="C", PYTHONHASHSEED="0", PYTHONNOUSERSITE="1")
    return env


def measure(command, expected, timeout):
    started = time.perf_counter_ns()
    try:
        child = subprocess.run(command, capture_output=True, env=environment(), timeout=timeout)
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(f"timeout after {timeout}s: {command}") from error
    elapsed = (time.perf_counter_ns() - started) / 1_000_000
    if child.returncode:
        raise RuntimeError(f"exit {child.returncode}: {command}: {child.stderr.decode('utf-8', errors='replace')[:1000]}")
    if child.stdout.replace(b"\r\n", b"\n") != expected.encode("utf-8"):
        raise RuntimeError(f"wrong output: {command}: {child.stdout[:200]!r}; expected {expected!r}")
    return elapsed


def resolve_commands(scripta, php, python):
    resolved = {name: shutil.which(str(value)) for name, value in
                (("scripta", scripta), ("php", php), ("python", python))}
    commands = {}
    unavailable = {}
    for name, path in resolved.items():
        if path is None:
            unavailable[name] = f"executable not found: {dict(scripta=scripta, php=php, python=python)[name]}"
        elif name == "php":
            commands[name] = [path, "-n", "-d", "opcache.enable_cli=0", "-d", "opcache.jit=0", "-d", "pcre.jit=0"]
        elif name == "python":
            commands[name] = [path, "-I"]
        else:
            commands[name] = [path, "run"]
    return commands, unavailable


def metadata(command):
    path = Path(command[0]).resolve()
    flag = "--version" if command[-1] == "run" else "-V" if "-I" in command else "-v"
    version = subprocess.check_output(command[:-1] + [flag] if command[-1] == "run" else command + [flag],
                                      env=environment(), timeout=10, text=True)
    return {"executable": str(path), "sha256": sha(path), "version": version.strip(), "command": command}


def cpu_name():
    if sys.platform == "darwin":
        return subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
    if sys.platform.startswith("linux"):
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    return platform.processor() or platform.machine()


def run(args):
    commands, unavailable = resolve_commands(args.scripta, args.php, args.python)
    report = {
        "schema": 1, "status": "running", "date_utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "machine": platform.machine(), "cpu": cpu_name(),
        "logical_cpus": os.cpu_count(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        "working_tree": subprocess.check_output(["git", "status", "--short"], cwd=REPO, text=True),
        "samples": args.samples, "warmups": args.warmups, "seed": args.seed,
        "timeout_seconds": args.timeout,
        "metric": "whole-process wall time in milliseconds; warm filesystem cache; startup included",
        "configuration": "PHP -n, CLI OPcache/JIT and PCRE JIT disabled; Python -I; SCR_* environment removed",
        "runtimes": {name: metadata(command) for name, command in commands.items()},
        "unavailable": unavailable, "results": [], "round_order": [],
        "fixture_sha256": {p.name: sha(p) for p in sorted((ROOT / "fixtures").iterdir()) if p.is_file()},
        "driver_sha256": sha(__file__),
        "compiler": subprocess.check_output(["rustc", "-vV"], cwd=REPO, text=True, timeout=10).strip(),
    }
    rng = random.Random(args.seed)
    extensions = {"php": "php", "python": "py", "scripta": "scripta"}
    try:
        for name, label, size, expected in WORKLOADS:
            observations = {language: [] for language in commands}
            for index in range(args.warmups + args.samples):
                order = list(commands)
                rng.shuffle(order)
                report["round_order"].append({"workload": name, "round": index, "languages": order})
                for language in order:
                    command = commands[language] + [str(ROOT / "fixtures" / f"{name}.{extensions[language]}")]
                    elapsed = measure(command, expected, args.timeout)
                    if index >= args.warmups:
                        observations[language].append(elapsed)
            for language, values in observations.items():
                result = {"workload": name, "label": label, "size": size, "language": language,
                          "observations_ms": values, "median_ms": statistics.median(values),
                          "min_ms": min(values), "max_ms": max(values)}
                report["results"].append(result)
                print(f"{name:18} {language:8} {result['median_ms']:10.3f} ms", flush=True)
        report["status"] = "partial" if unavailable else "complete"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        # Exclusive creation preserves historical evidence.
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            json.dump(report, output, indent=2)
            output.write("\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scripta", required=True)
    parser.add_argument("--php", default="php")
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--seed", type=int, default=1729)
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if args.samples < 1 or args.warmups < 0 or not 0 < args.timeout < float("inf"):
        parser.error("samples and timeout must be positive; warmups must be nonnegative")
    if args.output.exists():
        parser.error("output already exists; choose a new evidence filename")
    report = run(args)
    if report["status"] != "complete":
        parser.exit(1, f"incomplete comparison: {report['unavailable']}\n")


if __name__ == "__main__":
    main()
