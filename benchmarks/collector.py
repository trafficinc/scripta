#!/usr/bin/env python3
"""Measure collector pauses, scaling, execution cost, and isolated memory behavior."""
import argparse
import hashlib
import json
import math
import platform
import statistics
import subprocess
from pathlib import Path
from types import SimpleNamespace

from memory_soak import measure, positive

CASES = ("discarded", "live", "mixed", "recursive", "shared", "nested", "host",
         "plain_values", "ordinary", "closure_churn")


def identify(worker):
    return {"path": str(worker.resolve()),
            "sha256": hashlib.sha256(worker.read_bytes()).hexdigest()}


def run_case(worker, case, size, args):
    result = subprocess.run(
        [str(worker.resolve()), case, str(size), str(args.samples), str(args.warmups)],
        check=True, capture_output=True, text=True, timeout=args.timeout,
    )
    row = json.loads(result.stdout)
    if row["case"] != case or row["size"] != size or row["debug_assertions"]:
        raise RuntimeError("unexpected worker identity or unoptimized worker")
    observations = row["observations_ns"]
    if len(observations) != args.samples or len(row["reclaimed_scopes"]) != args.samples:
        raise RuntimeError("incomplete measurement samples")
    ordered = sorted(observations)
    row["median_ns"] = statistics.median(ordered)
    row["p95_ns"] = ordered[math.ceil(0.95 * len(ordered)) - 1]
    row["max_ns"] = ordered[-1]
    row["median_ns_per_item"] = row["median_ns"] / size
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, default=Path("target/release/examples/collector_bench"))
    parser.add_argument("--reference", type=Path, help="compatible preserved benchmark worker")
    parser.add_argument("--case", action="append", choices=CASES)
    parser.add_argument("--sizes", nargs="+", type=positive, default=[1000, 10000, 100000])
    parser.add_argument("--samples", type=positive, default=11)
    parser.add_argument("--warmups", type=positive, default=2)
    parser.add_argument("--timeout", type=positive, default=300, help="seconds per worker invocation")
    parser.add_argument("--soak-worker", type=Path, default=Path("target/release/examples/memory_soak"))
    parser.add_argument("--skip-memory", action="store_true")
    parser.add_argument("--memory-batches", type=positive, default=20)
    parser.add_argument("--memory-iterations", type=positive, default=500)
    parser.add_argument("--output", type=Path, required=True, help="new JSON file; never overwritten")
    args = parser.parse_args()
    required = [args.worker]
    if args.reference:
        required.append(args.reference)
    if not args.skip_memory:
        required.append(args.soak_worker)
    if any(not worker.is_file() for worker in required):
        parser.error("worker missing; build the optimized scr-runtime examples first")
    with args.output.open("x") as output:
        report = {
            "schema_version": 1, "platform": platform.platform(),
            "worker": identify(args.worker),
            "reference": identify(args.reference) if args.reference else None,
            "warmups": args.warmups, "samples": args.samples,
            "timing_thresholds_enforced": False, "measurements": [],
            "memory": None,
        }
        for case in args.case or CASES:
            for size in args.sizes:
                # Alternate order to reduce consistent candidate/reference ordering bias.
                workers = [("candidate", args.worker)]
                if args.reference:
                    workers.append(("reference", args.reference))
                    if len(report["measurements"]) % 2:
                        workers.reverse()
                pair = {name: run_case(worker, case, size, args) for name, worker in workers}
                if args.reference:
                    pair["median_ratio"] = (pair["candidate"]["median_ns"] /
                                            max(1, pair["reference"]["median_ns"]))
                report["measurements"].append(pair)
                row = pair["candidate"]
                print(f"{case} size={size}: median={row['median_ns']/1e6:.3f} ms "
                      f"p95={row['p95_ns']/1e6:.3f} ms max={row['max_ns']/1e6:.3f} ms",
                      flush=True)
        # Run RSS sampling after all timing trials to avoid sampling interference.
        if not args.skip_memory:
            memory_args = SimpleNamespace(
                worker=args.soak_worker, warmups=args.warmups, batches=args.memory_batches,
                iterations=args.memory_iterations, timeout=args.timeout,
            )
            report["memory"] = {
                "worker": identify(args.soak_worker),
                "warmup_batches": args.warmups, "batches": args.memory_batches,
                "iterations_per_batch": args.memory_iterations,
                "rss_method": {"Linux": "proc_status", "Darwin": "ps_rss"}.get(
                    platform.system(), "unavailable"),
                "workloads": [measure(memory_args, name) for name in
                              ("transient", "errors", "closures", "recreate")],
            }
        json.dump(report, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
