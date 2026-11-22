#!/usr/bin/env python3
"""Separate checked extended benchmarks; optional JSON, no published table updates."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

from workloads import CASES, expected, local_http, prepare, units

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
EXTENSIONS = {"scripta": "scripta", "php": "php", "python": "py"}
OUTPUT_LIMIT = 1024 * 1024
WAIT4 = getattr(os, "wait4", None)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def environment():
    env = {k: v for k, v in os.environ.items() if not k.startswith("SCR_") and
           k.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy")}
    env.update(LC_ALL="C", PYTHONHASHSEED="0", PYTHONNOUSERSITE="1", NO_PROXY="127.0.0.1,localhost")
    return env


def kill_group(child):
    # Signal delivery can race process exit. Retry briefly while reaping an
    # exited direct child; a persistent permission failure must still propagate.
    deadline = time.monotonic() + 0.1
    while True:
        try:
            os.killpg(child.pid, signal.SIGKILL)
            return
        except ProcessLookupError:
            return
        except PermissionError:
            child.poll()
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.001)


def stop(child):
    try:
        if os.name == "posix":
            kill_group(child)
        else:
            # Stop children launched by the subprocess workload too.
            try:
                result = subprocess.run(["taskkill", "/PID", str(child.pid), "/T", "/F"],
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
                if result.returncode and child.poll() is None:
                    child.kill()
            except (OSError, subprocess.TimeoutExpired):
                child.kill()
    finally:
        # Always reap the direct child, even when group cleanup is forbidden.
        try:
            if child.poll() is None:
                child.kill()
        finally:
            child.wait()


def measure(command, timeout):
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        start = time.perf_counter_ns()
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=output, stderr=errors,
                                 env=environment(), start_new_session=os.name == "posix")
        usage = None
        try:
            while True:
                if WAIT4 is not None:
                    pid, status, found = WAIT4(child.pid, os.WNOHANG)
                    if pid:
                        child.returncode = os.waitstatus_to_exitcode(status)
                        usage = found
                        break
                elif child.poll() is not None:
                    break
                if (time.perf_counter_ns() - start) / 1e9 > timeout:
                    raise RuntimeError(f"timeout after {timeout}s: {command}")
                if os.fstat(output.fileno()).st_size + os.fstat(errors.fileno()).st_size > OUTPUT_LIMIT:
                    raise RuntimeError("worker output limit exceeded")
                time.sleep(0.001)
        except BaseException:
            stop(child)
            raise
        wall = (time.perf_counter_ns() - start) / 1e6
        # A failed fixture may exit while a grandchild still holds resources.
        # Workers own their session; no legitimate work survives a worker exit.
        if os.name == "posix":
            kill_group(child)
        output.seek(0)
        errors.seek(0)
        stdout, stderr = output.read(OUTPUT_LIMIT + 1), errors.read(OUTPUT_LIMIT + 1)
        if len(stdout) + len(stderr) > OUTPUT_LIMIT:
            raise RuntimeError("worker output limit exceeded")
        if child.returncode:
            raise RuntimeError(f"exit {child.returncode}: {stderr.decode('utf-8', errors='replace')[:1000]}")
        if stderr:
            raise RuntimeError(f"unexpected stderr: {stderr.decode('utf-8', errors='replace')[:1000]}")
        rss = None if usage is None else usage.ru_maxrss / (1024 * 1024 if sys.platform == "darwin" else 1024)
        return {"wall_ms": wall, "peak_rss_mib": rss}, stdout


def validate(stdout, case, iterations, batches, wall_ms):
    if case == "startup":
        if stdout:
            raise RuntimeError("startup output must be empty")
        return None
    try:
        payload = json.loads(stdout)
        times = payload["elapsed_ms"]
        checksums = payload["checksums"]
        valid_times = (isinstance(times, list) and len(times) == batches and
                       all(type(v) in (float, int) and math.isfinite(v) and v >= 0 for v in times))
        checksum = expected(case, iterations)
        valid_checksums = (isinstance(checksums, list) and len(checksums) == batches and
                           all(type(v) is int and v == checksum for v in checksums))
        if (payload["case"] != case or not valid_times or not valid_checksums or
                sum(times) <= 0 or sum(times) > wall_ms + 5):
            raise ValueError("invalid batch timings, workload, or checksum")
    except (ValueError, KeyError, TypeError) as error:
        raise RuntimeError(f"invalid worker output for {case}: {error}") from error
    return times


def resolve(args):
    commands, missing = {}, {}
    for name in EXTENSIONS:
        requested = getattr(args, name)
        path = shutil.which(str(requested))
        if path is None:
            missing[name] = f"executable not found: {requested}"
        elif name == "php":
            commands[name] = [path, "-n", "-d", "opcache.enable_cli=0", "-d", "opcache.jit=0", "-d", "pcre.jit=0"]
            for extension in args.php_extension:
                commands[name].extend(["-d", "extension=" + extension])
        elif name == "python":
            commands[name] = [path, "-I"]
        else:
            commands[name] = [path, "run"]
    return commands, missing


def sqlite_probe(command, language):
    if language == "scripta":
        return command + [str(ROOT / "fixtures/sqlite_probe.scripta")]
    if language == "python":
        return command + ["-c", "import sqlite3"]
    return command + ["-r", 'exit(class_exists("SQLite3") ? 0 : 3);']


def metadata(command, language):
    version_command = (command[:-1] + ["--version"] if language == "scripta" else
                       command + ["-V" if language == "python" else "-v"])
    version = subprocess.check_output(version_command, env=environment(), timeout=10, text=True)
    return {"command": command, "sha256": sha(command[0]), "version": version.strip()}


def summarize(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def run(args):
    if args.output and args.output.exists():
        raise ValueError("output already exists; choose a new filename")
    commands, missing = resolve(args)
    cases = list(dict.fromkeys(args.workload or CASES))
    report = {
        "schema": 1, "suite": "extended", "status": "running",
        "date_utc": datetime.now(timezone.utc).isoformat(), "platform": platform.platform(),
        "machine": platform.machine(), "logical_cpus": os.cpu_count(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        "working_tree": subprocess.check_output(["git", "status", "--short"], cwd=REPO, text=True),
        "settings": {k: getattr(args, k) for k in ("samples", "warmups", "seed", "iterations", "batches", "timeout", "php_extension")},
        "configuration": "PHP -n, CLI OPcache/JIT and PCRE JIT off; Python -I; SCR_* and proxy environment removed. SQLite Python statement cache disabled; all languages prepare inserts per call in one transaction per batch.",
        "execution_metric": "Worker monotonic-clock milliseconds around each work() call, including IO and in-worker checks, excluding process launch/parsing/imports, final report encoding, and parent validation. No startup subtraction.",
        "startup_metric": "Whole-process empty-script wall time until observed exit, reported separately; completion polled at 1 ms. All launches retain whole-process wall time.",
        "rss_method": "wait4 per-child ru_maxrss; bytes on macOS, KiB elsewhere; converted to MiB; OS descendant accounting may apply. Excludes parent HTTP server; whole-process high-water mark, not current heap or leak evidence." if WAIT4 is not None else "unavailable on this host; correctness and timings still run",
        "cleanup_method": "POSIX process-group kill on timeout/failure" if os.name == "posix" else "Windows taskkill /T /F on timeout; descendant cleanup after early parent exit is unavailable; native Windows validation pending",
        "runtimes": {}, "unavailable": missing, "results": [], "round_order": [],
        "fixture_sha256": {p.name: sha(p) for p in sorted((ROOT / "fixtures").iterdir()) if p.is_file()},
        "driver_sha256": {p.name: sha(p) for p in (Path(__file__), ROOT / "workloads.py")},
        "compiler": subprocess.check_output(["rustc", "-vV"], cwd=REPO, text=True).strip(),
    }
    rng = random.Random(args.seed)
    try:
        report["runtimes"] = {name: metadata(command, name) for name, command in commands.items()}
        sqlite_missing = {}
        if "sqlite" in cases:
            for language, command in commands.items():
                try:
                    measure(sqlite_probe(command, language), args.timeout)
                except RuntimeError as error:
                    sqlite_missing[language] = str(error)
            if sqlite_missing:
                report["unavailable"]["sqlite"] = sqlite_missing
        if "subprocess" in cases and "python" not in commands:
            report["unavailable"]["subprocess"] = "Python helper executable unavailable for every language"
        with tempfile.TemporaryDirectory(prefix="scripta extended ") as temporary:
            directory = Path(temporary)
            expected_file = prepare(directory)
            report["input_sha256"] = {p.name: sha(p) for p in sorted(directory.iterdir())}
            with local_http("http_stream" in cases and bool(commands)) as (url, server):
                for case in cases:
                    available = {k: v for k, v in commands.items() if
                                 not (case == "sqlite" and k in sqlite_missing) and
                                 not (case == "subprocess" and "python" not in commands)}
                    observations = {name: [] for name in available}
                    rows = {}
                    for language in available:
                        count, unit = units(case, args.iterations)
                        rows[language] = {"workload": case, "language": language,
                                          "units_per_batch": count, "unit": unit,
                                          "observations": observations[language]}
                        report["results"].append(rows[language])
                    http_before = server.requests if server else 0
                    for index in range(args.warmups + args.samples):
                        order = list(available)
                        rng.shuffle(order)
                        report["round_order"].append({"workload": case, "round": index, "languages": order})
                        for language in order:
                            report["active"] = {"workload": case, "language": language, "round": index}
                            fixture = ROOT / "fixtures" / (("startup" if case == "startup" else "worker") + "." + EXTENSIONS[language])
                            command = available[language] + [str(fixture)]
                            if case != "startup":
                                helper = commands.get("python", [sys.executable])[0]
                                command += [case, str(args.iterations), str(args.batches), str(directory), url, helper]
                            observation, stdout = measure(command, args.timeout)
                            times = validate(stdout, case, args.iterations, args.batches, observation["wall_ms"])
                            if case == "files" and (directory / ("output-" + language + ".txt")).read_bytes() != expected_file:
                                raise RuntimeError("transformed file content mismatch")
                            observation["execution_batches_ms"] = times
                            observation["warmup"] = index < args.warmups
                            observations[language].append(observation)
                    if case == "http_stream" and server is not None and server.requests - http_before != len(available) * (args.warmups + args.samples) * args.batches:
                        raise RuntimeError("HTTP request count mismatch")
                    for language, raw in observations.items():
                        measured = [o for o in raw if not o["warmup"]]
                        count, unit = units(case, args.iterations)
                        row = rows[language]
                        row["wall_ms"] = summarize([o["wall_ms"] for o in measured])
                        if case != "startup":
                            per_batch = [sum(o["execution_batches_ms"]) / args.batches for o in measured]
                            row["execution_ms_per_batch"] = summarize(per_batch)
                            row["units_per_second"] = count * 1000 / statistics.median(per_batch)
                        rss = [o["peak_rss_mib"] for o in measured if o["peak_rss_mib"] is not None]
                        row["peak_rss_mib"] = summarize(rss) if rss else None
                        timing = row.get("execution_ms_per_batch", row["wall_ms"])["median"]
                        rate = f" | {row['units_per_second']:.0f} {unit}/s" if case != "startup" else ""
                        memory = f"{row['peak_rss_mib']['median']:.2f} MiB" if rss else "unavailable"
                        print(f"{case:18} {language:8} {timing:10.3f} ms" +
                              (" / batch" if case != "startup" else " / launch") +
                              rate + " | peak RSS " + memory, flush=True)
                if server is not None:
                    report["http_fixture"] = {"requests": server.requests, "chunk_bytes": 1024,
                                              "payload_bytes": (directory / "payload.bin").stat().st_size}
        report.pop("active", None)
        report["status"] = "partial" if report["unavailable"] else "complete"
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            with args.output.open("x", encoding="utf-8", newline="\n") as output:
                json.dump(report, output, indent=2, allow_nan=False)
                output.write("\n")
    return report


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--scripta", required=True)
    result.add_argument("--php", default="php")
    result.add_argument("--python", default=sys.executable)
    result.add_argument("--php-extension", action="append", default=[])
    result.add_argument("--output", type=Path, help="optional new JSON path; default prints only timings")
    result.add_argument("--workload", action="append", choices=CASES)
    result.add_argument("--samples", type=int, default=5)
    result.add_argument("--warmups", type=int, default=1)
    result.add_argument("--seed", type=int, default=1729)
    result.add_argument("--iterations", type=int, default=20000)
    result.add_argument("--batches", type=int, default=3)
    result.add_argument("--timeout", type=float, default=60)
    return result


def main():
    cli = parser()
    args = cli.parse_args()
    for name, low, high in (("samples", 1, 100), ("warmups", 0, 20), ("iterations", 1, 1000000), ("batches", 1, 10000)):
        if not low <= getattr(args, name) <= high:
            cli.error(f"{name} must be in {low}..{high}")
    if not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        cli.error("timeout must be finite and in (0, 3600]")
    if args.output and args.output.exists():
        cli.error("output already exists; choose a new filename")
    report = run(args)
    if report["status"] != "complete":
        cli.exit(1, f"incomplete extended comparison: {report['unavailable']}\n")


if __name__ == "__main__":
    main()
