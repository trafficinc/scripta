#!/usr/bin/env python3
"""Measure parsed-program execution separately from CLI launch; preserve every sample."""
import argparse
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import statistics
import subprocess
import tempfile

from run import REPO, ROOT, CORE_WORKLOADS as WORKLOADS, environment, sha

STAGES = ("lex_ms", "parse_ms", "setup_ms", "execute_ms", "teardown_ms")


def validate_observations(result, samples):
    if not isinstance(result, dict) or set(result) != set(STAGES):
        raise ValueError("worker stage matrix is incomplete or unexpected")
    for values in result.values():
        if not isinstance(values, list) or len(values) != samples:
            raise ValueError("worker returned an incorrect sample count")
        if any(isinstance(value, bool) or not isinstance(value, (int, float))
               or not math.isfinite(value) or value < 0 for value in values):
            raise ValueError("worker returned invalid timing observations")
    return {stage: {"observations_ms": values, "median_ms": statistics.median(values),
                    "min_ms": min(values), "max_ms": max(values)}
            for stage, values in result.items()}


def worker_command(worker, fixture, expected, samples, warmups, mode="measure", seconds=5):
    return [str(worker), str(fixture), str(expected), str(samples), str(warmups), mode, str(seconds)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--baseline-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=3)
    args = parser.parse_args()
    if not 1 <= args.samples <= 10000 or not 0 <= args.warmups <= 10000:
        parser.error("samples must be 1..10000; warmups must be 0..10000")
    if args.output.exists():
        parser.error("choose a new output file to preserve existing evidence")
    worker = args.worker.resolve()
    baseline = json.loads(args.baseline_manifest.read_text())
    if sha(REPO / baseline["binary"]) != baseline["binary_sha256"]:
        parser.error("preserved baseline binary hash mismatch")
    report = {
        "schema": 1, "status": "running", "date_utc": datetime.now(timezone.utc).isoformat(),
        "metric": "parsed-program API; execution excludes lexing, parsing, runtime setup and teardown",
        "scope": "fresh runtime per sample; output and completion validated; no attached source text, module resolver, or CLI process setup",
        "worker": str(worker), "worker_sha256": sha(worker),
        "worker_source_sha256": sha(REPO / "crates/scr-cli/examples/comparison_profile.rs"),
        "driver_sha256": sha(__file__), "baseline_manifest": baseline,
        "samples": args.samples, "warmups": args.warmups, "results": [],
        "worker_build_recipe": "release with CARGO_PROFILE_RELEASE_DEBUG=1 for native stack symbols; default SQLite feature",
    }
    try:
        with tempfile.TemporaryDirectory(prefix="scripta-execution-profile-") as directory:
            expected = Path(directory) / "expected.txt"
            for name, _, _, output in WORKLOADS:
                fixture = ROOT / "fixtures" / f"{name}.scripta"
                if sha(fixture) != baseline["fixture_sha256"][fixture.name]:
                    raise ValueError(f"fixture changed since baseline: {name}")
                expected.write_bytes(output.encode())
                command = worker_command(worker, fixture, expected, args.samples, args.warmups)
                child = subprocess.run(command, capture_output=True, env=environment(), timeout=300)
                if child.returncode:
                    raise RuntimeError(f"worker exit {child.returncode}: {child.stderr.decode(errors='replace')[:1000]}")
                stages = validate_observations(json.loads(child.stdout), args.samples)
                report["results"].append({"workload": name, "fixture_sha256": sha(fixture), "stages": stages})
                print(f"{name:18} execute {stages['execute_ms']['median_ms']:10.3f} ms", flush=True)
        report["status"] = "complete"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8", newline="\n") as output:
            json.dump(report, output, indent=2)
            output.write("\n")


if __name__ == "__main__":
    main()
