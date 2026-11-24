#!/usr/bin/env python3
"""macOS native stack/heap capture; intentionally separate from benchmark timing."""
import argparse
from datetime import datetime, timezone
import json
import re
from pathlib import Path
import select
import subprocess
import sys
import tempfile

from execution import worker_command
from run import ROOT, CORE_WORKLOADS as WORKLOADS, environment, sha


def allocation_totals(text):
    operations = re.search(r"Total heap \+ VM memory operations:\s*([\d,]+)\s+allocations:\s*([\d,]+)\s+deallocations:\s*([\d,]+)", text)
    cumulative = re.search(r"Cumulative total allocated heap \+ VM memory:\s*([^\n]+)", text)
    if operations is None or cumulative is None:
        raise ValueError("native allocation report lacks recorded lifetime totals")
    return {"recorded_operations": int(operations[1].replace(",", "")),
            "recorded_allocations": int(operations[2].replace(",", "")),
            "recorded_deallocations": int(operations[3].replace(",", "")),
            "recorded_cumulative_size": cumulative[1].strip()}


def capture(worker, name, expected_output, kind, directory):
    with tempfile.TemporaryDirectory(prefix="scripta-native-profile-") as temporary:
        expected = Path(temporary) / "expected.txt"
        expected.write_bytes(expected_output.encode())
        env = environment()
        if kind == "alloc":
            env["MallocStackLogging"] = "full"
            env["MallocStackLoggingDirectory"] = str(directory.resolve())
        command = worker_command(worker, ROOT / "fixtures" / f"{name}.scripta", expected,
                                 1, 0, mode=kind, seconds=15 if kind == "cpu" else 120)
        with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env) as child:
            try:
                if not select.select([child.stderr], [], [], 90)[0]:
                    raise RuntimeError("worker did not become ready")
                # MallocStackLogging can print startup notices before the ready marker.
                notices = []
                while True:
                    line = child.stderr.readline().decode(errors="replace").strip()
                    if line == "READY":
                        break
                    if not line:
                        raise RuntimeError(f"worker exited before readiness: {notices}")
                    notices.append(line)
                raw = directory / f"{name}-{kind}.txt"
                if kind == "cpu":
                    native = ["/usr/bin/sample", str(child.pid), "3", "1", "-mayDie", "-file", str(raw)]
                    result = subprocess.run(native, capture_output=True, text=True, timeout=90)
                else:
                    native = ["/usr/bin/malloc_history", str(child.pid), "-highWaterMark", "-callTree", "-ignoreThreads", "-collapseRecursion", "-noContent"]
                    result = subprocess.run(native, capture_output=True, text=True, timeout=90)
                    raw.write_text(result.stdout)
                if result.returncode:
                    raise RuntimeError(f"native profiler exit {result.returncode}: {result.stderr[:2000]}")
                if not raw.is_file() or raw.stat().st_size == 0:
                    raise RuntimeError("native profiler produced no data")
                allocation_data = {}
                if kind == "alloc":
                    allocation_data = allocation_totals(raw.read_text())
                    live = directory / f"{name}-alloc-live.txt"
                    live_command = ["/usr/bin/malloc_history", str(child.pid), "-callTree", "-ignoreThreads", "-collapseRecursion", "-noContent"]
                    live_result = subprocess.run(live_command, capture_output=True, text=True, timeout=90)
                    if live_result.returncode:
                        raise RuntimeError(f"live allocation capture failed: {live_result.stderr[:1000]}")
                    live.write_text(live_result.stdout)
                    allocation_data.update(live_raw=str(live), live_raw_sha256=sha(live))
                if kind == "cpu":
                    stdout, stderr = child.communicate(timeout=90)
                    if child.returncode:
                        raise RuntimeError(f"profile worker failed: {stderr.decode(errors='replace')[:1000]}")
                    worker_result = json.loads(stdout)
                else:
                    # The validated execution completed before READY; stop its inspection hold.
                    child.terminate()
                    child.communicate(timeout=10)
                    worker_result = {"mode": "alloc", "iterations": 1,
                                     "stopped_after": "validated execution and native heap inspection"}
                return {"workload": name, "kind": kind, "raw": str(raw), "raw_sha256": sha(raw),
                        "worker": worker_result, "notices": notices, "command": native,
                        "profiler_stderr": result.stderr, "allocation_totals": allocation_data}
            finally:
                if child.poll() is None:
                    child.kill()
                    child.communicate(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--kind", choices=("cpu", "alloc"), required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--workload", choices=[row[0] for row in WORKLOADS], action="append")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this native profiling driver requires macOS; timing worker is portable")
    if args.directory.exists():
        parser.error("choose a new capture directory")
    args.directory.mkdir(parents=True)
    worker = args.worker.resolve()
    report = {"schema": 1, "status": "running", "kind": args.kind,
              "date_utc": datetime.now(timezone.utc).isoformat(),
              "worker_sha256": sha(worker), "driver_sha256": sha(__file__), "results": [],
              "scope": "sampled stacks during repeated fresh-runtime executions" if args.kind == "cpu" else
                       "native peak/live allocation stacks and recorded lifetime heap+VM totals; includes launch/setup and profiler overhead; compacted logs can omit immediate alloc/free pairs"}
    try:
        for name, _, _, expected in WORKLOADS:
            if args.workload and name not in args.workload:
                continue
            report["results"].append(capture(worker, name, expected, args.kind, args.directory))
            print(f"Captured {args.kind}: {name}", flush=True)
        report["status"] = "complete"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        (args.directory / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
