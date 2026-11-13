#!/usr/bin/env python3
"""Sample current RSS of isolated runtime workloads; no third-party dependencies."""
import argparse
import hashlib
import json
import platform
import queue
import subprocess
import threading
import time
from pathlib import Path

WORKLOADS = ("transient", "errors", "closures", "recreate")


def positive(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def current_rss(pid):
    if platform.system() == "Linux":
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
        raise RuntimeError("VmRSS missing from child status")
    if platform.system() == "Darwin":
        result = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)],
            check=True, capture_output=True, text=True, timeout=5,
        )
        return int(result.stdout.strip()) * 1024
    return None


def measure(args, workload):
    child = subprocess.Popen(
        [str(args.worker.resolve()), workload, str(args.warmups),
         str(args.batches), str(args.iterations)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    )
    messages = queue.Queue()

    def read_checkpoints():
        try:
            for line in child.stdout:
                messages.put(line)
        finally:
            messages.put(None)

    reader = threading.Thread(target=read_checkpoints, daemon=True)
    reader.start()
    samples = []
    started = time.monotonic()
    try:
        expected = [("baseline", 0)]
        expected += [("sample", batch) for batch in range(1, args.batches + 1)]
        expected += [("dropped", args.batches)]
        for phase, batch in expected:
            line = messages.get(timeout=args.timeout)
            if line is None or line.strip() != f"{phase} {batch}":
                raise RuntimeError(f"{workload}: unexpected checkpoint {line!r}")
            samples.append({
                "phase": phase, "batch": batch,
                "elapsed_seconds": time.monotonic() - started,
                "rss_bytes": current_rss(child.pid),
            })
            child.stdin.write("next\n")
            child.stdin.flush()
        child.stdin.close()
        if child.wait(timeout=args.timeout) != 0:
            raise RuntimeError(f"{workload}: worker failed")
        if messages.get(timeout=args.timeout) is not None:
            raise RuntimeError(f"{workload}: extra worker output")
    finally:
        if child.poll() is None:
            child.kill()
        child.wait()
        reader.join(timeout=5)
        child.stdout.close()
        if not child.stdin.closed:
            child.stdin.close()
    baseline = samples[0]["rss_bytes"]
    measured = samples[1:-1]
    growth = None if baseline is None else measured[-1]["rss_bytes"] - baseline
    return {
        "workload": workload,
        "expected_behavior": (
            "Captured environments remain until runtime drop; growth is expected."
            if workload == "closures" else
            "Memory should settle after warmup; allocator retention may remain."
        ),
        "operations_per_batch": args.iterations * (2 if workload == "errors" else 1),
        "growth_from_baseline_bytes": growth,
        "samples": samples,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, default=Path("target/release/examples/memory_soak"))
    parser.add_argument("--workload", action="append", choices=WORKLOADS,
                        help="repeat to select workloads; default runs all four")
    parser.add_argument("--warmups", type=positive, default=3)
    parser.add_argument("--batches", type=positive, default=20)
    parser.add_argument("--iterations", type=positive, default=500)
    parser.add_argument("--timeout", type=positive, default=120, help="seconds per checkpoint")
    parser.add_argument("--output", type=Path, required=True, help="new JSON file; never overwritten")
    args = parser.parse_args()
    if not args.worker.is_file():
        parser.error("worker not found; build the scr-runtime memory_soak example first")
    # Reserve the output before starting expensive work, and retain earlier runs.
    with args.output.open("x") as output:
        report = {
            "schema_version": 1,
            "platform": platform.platform(),
            "worker": str(args.worker.resolve()),
            "worker_sha256": hashlib.sha256(args.worker.read_bytes()).hexdigest(),
            "warmup_batches": args.warmups,
            "measured_batches": args.batches,
            "iterations_per_batch": args.iterations,
            "rss_method": {"Linux": "proc_status", "Darwin": "ps_rss"}.get(
                platform.system(), "unavailable"),
            "thresholds_enforced": False,
            "workloads": [measure(args, name) for name in (args.workload or WORKLOADS)],
        }
        json.dump(report, output, indent=2)
        output.write("\n")
    for result in report["workloads"]:
        print(f"{result['workload']}: RSS growth {result['growth_from_baseline_bytes']} bytes")


if __name__ == "__main__":
    main()
