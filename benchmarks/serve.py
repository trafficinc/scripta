#!/usr/bin/env python3
"""Bounded loopback HTTP measurements and recovery evidence; see serve.md."""
import argparse
import collections
import concurrent.futures
import datetime
import hashlib
import http.client
import json
import math
import os
import platform
import signal
import sqlite3
import subprocess
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PAYLOAD = bytes(range(256)) * 1024
STREAM = b"".join(f"chunk-{i}\n".encode() for i in range(4))
LOG_LIMIT = 256 * 1024 * 1024


def command(args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True,
                          text=True, timeout=15).stdout.strip()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bounded(low, high):
    def parse(value):
        number = int(value)
        if not low <= number <= high:
            raise argparse.ArgumentTypeError(f"must be {low}..{high}")
        return number
    return parse


def distribution(values):
    if not values:
        return None
    values = sorted(values)
    return {"count": len(values), "min": values[0], "max": values[-1],
            **{f"p{p}": values[max(0, math.ceil(len(values) * p / 100) - 1)]
               for p in (50, 95, 99)}}


def events(path):
    if path.stat().st_size > LOG_LIMIT:
        raise RuntimeError("fixture log exceeded 256 MiB ceiling")
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            if len(line) > 32768:
                raise RuntimeError("fixture log frame exceeded ceiling")
            if line.endswith("\n"):
                yield json.loads(line)


class Server:
    def __init__(self, binary, directory):
        self.directory = directory
        self.log_path = directory / "server.log"
        self.log = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [str(binary), "serve", str(ROOT / "benchmarks/serve.scripta"),
             "--host", "127.0.0.1", "--port", "0", "--workers", "2",
             "--queue", "8", "--max-connections", "64", "--timeout-ms", "750",
             "--connection-timeout-ms", "120000", "--drain-timeout-ms", "1500",
             "--worker-max-requests", "200", "--log-format", "json",
             "--health-address", "127.0.0.1:0", "--upload-temp-dir", "spools",
             "--session-dir", "sessions"],
            cwd=directory, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=self.log,
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0,
            start_new_session=os.name != "nt",
        )
        self.address = None
        self.health_address = None
        self.closed = False
        self.cleanup = None

    def start(self):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            for event in events(self.log_path):
                if event["event"] == "server_listening":
                    self.address = event["address"]
                elif event["event"] == "health_listening":
                    self.health_address = event["address"]
            if self.address and self.health_address:
                self.recovered()
                return self
            if self.process.poll() is not None:
                raise RuntimeError("server exited during startup")
            time.sleep(0.02)
        raise RuntimeError("server startup deadline exceeded")

    def metrics(self):
        connection = http.client.HTTPConnection(self.health_address, timeout=3)
        try:
            connection.request("GET", "/metrics")
            response = connection.getresponse()
            if response.status != 200:
                raise RuntimeError("private metrics failed")
            return json.loads(response.read(65536))
        finally:
            connection.close()

    def recovered(self):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            metrics = self.metrics()
            if (metrics["ready"] and metrics["readyWorkers"] == 2
                    and all(metrics[name] == 0 for name in
                            ("activeRequests", "queuedRequests", "busyWorkers", "recoveringWorkers"))):
                if list((self.directory / "spools").iterdir()):
                    raise RuntimeError("request spools survived recovery")
                return metrics
            time.sleep(0.025)
        raise RuntimeError("full idle capacity failed to recover")

    def close(self):
        if self.closed:
            return self.cleanup
        self.closed = True
        graceful = True
        if self.process.poll() is None:
            self.process.send_signal(signal.CTRL_BREAK_EVENT if os.name == "nt" else signal.SIGTERM)
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            graceful = False
            self.process.kill()
            self.process.wait(timeout=5)
        self.log.close()
        counts = collections.Counter()
        pids = set()
        stopped = None
        for event in events(self.log_path):
            counts[event["event"]] += 1
            if len(counts) > 64:
                raise RuntimeError("unexpected event cardinality")
            if event["event"] == "worker_started":
                pids.add(event["workerPid"])
                if len(pids) > 10000:
                    raise RuntimeError("fixture worker count ceiling exceeded")
            elif event["event"] == "server_stopped":
                stopped = event
        alive = []
        if os.name != "nt":
            for pid in pids:
                try:
                    os.kill(pid, 0)
                    alive.append(pid)
                except ProcessLookupError:
                    pass
        self.cleanup = {"graceful": graceful, "exit_code": self.process.returncode,
                        "event_counts": dict(counts), "workers_observed": len(pids),
                        "surviving_worker_pids": alive if os.name != "nt" else None,
                        "remaining_spools": len(list((self.directory / "spools").iterdir())),
                        "stopped_metrics": stopped}
        if (not graceful or self.process.returncode != 0 or stopped is None or alive
                or self.cleanup["remaining_spools"] != 0
                or any(stopped[name] != 0 for name in
                       ("activeRequests", "queuedRequests", "busyWorkers", "readyWorkers", "recoveringWorkers"))):
            raise RuntimeError("server shutdown/owned-resource cleanup failed")
        return self.cleanup


class Sampler:
    def __init__(self, server):
        self.server = server
        self.samples = []
        self.errors = []
        self.stop = threading.Event()
        self.started = time.monotonic()
        self.phase = "startup"
        self.thread = threading.Thread(target=self.run, daemon=True)

    def run(self):
        while not self.stop.is_set():
            try:
                sample = {"seconds": time.monotonic() - self.started, "phase": self.phase,
                          "supervisor_rss_bytes": None, "workers_rss_bytes": None,
                          "worker_count": None}
                if platform.system() in ("Darwin", "Linux"):
                    result = subprocess.run(["ps", "-axo", "pid=,ppid=,rss="], check=True,
                                            capture_output=True, text=True, timeout=3)
                    rows = [tuple(map(int, line.split())) for line in result.stdout.splitlines()]
                    sample["supervisor_rss_bytes"] = next(
                        rss * 1024 for pid, _, rss in rows if pid == self.server.process.pid)
                    workers = [rss * 1024 for _, parent, rss in rows if parent == self.server.process.pid]
                    sample["workers_rss_bytes"] = sum(workers)
                    sample["worker_count"] = len(workers)
                sample["metrics"] = self.server.metrics()
                self.samples.append(sample)
                if len(self.samples) > 2000:
                    raise RuntimeError("sampler exceeded 2000 observations")
            except Exception as error:
                self.errors.append(type(error).__name__ + ": " + str(error))
                return
            self.stop.wait(0.5)

    def finish(self):
        self.stop.set()
        self.thread.join(timeout=5)
        if self.thread.is_alive() or self.errors:
            raise RuntimeError("memory/metrics sampler failed: " + str(self.errors))


def exchange(connection, route, soak=False):
    started = time.perf_counter()
    upload = route in ("/upload", "/abandon")
    connection.request("POST" if upload else "GET", route,
                       body=PAYLOAD if upload else None,
                       headers={"Content-Type": "application/octet-stream"})
    response = connection.getresponse()
    first = response.read(1)
    first_ms = (time.perf_counter() - started) * 1000
    if route == "/paced":
        raise RuntimeError("paced route must use download")
    body = first
    if getattr(connection, "paced", False):
        chunks = [first]
        while True:
            chunk = response.read(4096)
            if not chunk:
                break
            chunks.append(chunk)
            time.sleep(0.002)
        body = b"".join(chunks)
    else:
        body += response.read(9 * 1024 * 1024)
    elapsed_ms = (time.perf_counter() - started) * 1000
    allowed = {200, 503} if route == "/slow" else {200, 503, 504} if soak else {200}
    if response.status not in allowed:
        raise RuntimeError(f"{route}: unexpected HTTP {response.status}")
    if response.status == 200:
        if route == "/json" and json.loads(body) != {"ok": True, "path": "/json"}:
            raise RuntimeError("incorrect JSON response")
        if route == "/sqlite" and not json.loads(body)["total"] > 0:
            raise RuntimeError("incorrect SQLite counter")
        if route == "/hash" and json.loads(body) != {"verified": True}:
            raise RuntimeError("password hash failed verification")
        expected = {"/upload": PAYLOAD, "/download": PAYLOAD, "/stream": STREAM, "/slow": b"ok"}
        if route in expected and body != expected[route]:
            raise RuntimeError(f"{route}: incorrect binary/stream output")
    return response.status, elapsed_ms, first_ms, len(body)


def phase(server, sampler, name, route, seconds, concurrency, ceiling, soak=False, paced=False):
    sampler.phase = name
    before = server.recovered()
    for _ in range(2):
        connection = http.client.HTTPConnection(server.address, timeout=5)
        try:
            exchange(connection, route)
        finally:
            connection.close()
    began = time.monotonic()
    deadline = began + seconds
    lock = threading.Lock()
    issued = 0

    def client():
        nonlocal issued
        connection = http.client.HTTPConnection(server.address, timeout=5)
        connection.paced = paced
        rows = []
        try:
            while time.monotonic() < deadline:
                with lock:
                    if issued >= ceiling:
                        break
                    issued += 1
                rows.append(exchange(connection, route, soak))
        finally:
            connection.close()
        return rows

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(client) for _ in range(concurrency)]
        rows = [row for future in futures for row in future.result()]
    elapsed = time.monotonic() - began
    after = server.recovered()
    statuses = collections.Counter(str(row[0]) for row in rows)
    return {"name": name, "seconds": elapsed, "requested_seconds": seconds,
            "clients": concurrency, "request_ceiling_reached": issued >= ceiling,
            "responses": len(rows), "statuses": dict(statuses),
            "responses_per_second": len(rows) / elapsed,
            "successful_responses_per_second": statuses["200"] / elapsed,
            "received_body_bytes": sum(row[3] for row in rows),
            "latency_ms": distribution([row[1] for row in rows]),
            "success_latency_ms": distribution([row[1] for row in rows if row[0] == 200]),
            "first_body_byte_ms": distribution([row[2] for row in rows if row[0] == 200]),
            "before": before, "after_recovery": after}


def fault_loop(server, stop, record, errors):
    routes = ("/failure", "/timeout", "/abandon")
    index = 0
    try:
        while not stop.is_set():
            route = routes[index % len(routes)]
            index += 1
            connection = http.client.HTTPConnection(server.address, timeout=5)
            try:
                connection.request("POST" if route == "/abandon" else "GET", route,
                                   body=PAYLOAD if route == "/abandon" else None)
                response = connection.getresponse()
                expected = 200 if route == "/abandon" else 500 if route == "/failure" else 504
                if response.status != expected:
                    raise RuntimeError(f"fault {route}: expected {expected}, got {response.status}")
                if route == "/abandon":
                    if not response.read(1):
                        raise RuntimeError("disconnect fixture failed to start streaming")
                else:
                    response.read(65536)
                record[route] += 1
            finally:
                connection.close()
            stop.wait(0.5)
    except Exception as error:
        errors.append(str(error))


def run(args):
    binary = args.scripta.resolve()
    report = {"schema": 1, "protocol": "benchmarks/serve.md", "status": "running",
              "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "revision": command(["git", "rev-parse", "HEAD"]),
              "working_tree": command(["git", "status", "--porcelain"]),
              "tracked_diff_sha256": hashlib.sha256(subprocess.run(
                  ["git", "diff", "--binary"], cwd=ROOT, check=True, capture_output=True, timeout=15).stdout).hexdigest(),
              "host": platform.platform(), "machine": platform.machine(), "logical_cpus": os.cpu_count(),
              "python": platform.python_version(), "rustc": command(["rustc", "-vV"]),
              "binary_sha256": sha(binary), "binary_bytes": binary.stat().st_size,
              "driver_sha256": sha(Path(__file__)), "protocol_sha256": sha(ROOT / "benchmarks/serve.md"), "fixture_sha256": sha(ROOT / "benchmarks/serve.scripta"),
              "parameters": {"seconds": args.seconds, "soak_seconds": args.soak_seconds,
                             "request_ceiling": args.max_requests, "workers": 2, "queue": 8,
                             "normal_clients": 4, "overload_clients": 16, "recycle_requests": 200,
                             "json_scaling_clients": [1, 8, 32] if args.scaling else [],
                             "request_timeout_ms": 750, "binary_bytes": len(PAYLOAD),
                             "runtime_overrides": {name: ("unset" if os.getenv(name) is None else
                                 int(os.environ[name]) if os.environ[name].isdigit() else "invalid")
                                 for name in ("SCR_MAX_STEPS", "SCR_MAX_VALUE_BYTES", "SCR_MAX_SCOPES", "SCR_MAX_SOURCE_BYTES")}},
              "workloads": []}
    with tempfile.TemporaryDirectory(prefix="scripta-http-bench-") as temporary:
        directory = Path(temporary)
        (directory / "payload.bin").write_bytes(PAYLOAD)
        (directory / "chunk.txt").write_text("x" * 65536, encoding="ascii")
        database = sqlite3.connect(str(directory / "work.sqlite"))
        database.execute("CREATE TABLE visits (id INTEGER PRIMARY KEY, total INTEGER NOT NULL)")
        database.execute("INSERT INTO visits VALUES (1, 0)")
        database.commit()
        database.close()
        server = Server(binary, directory)
        sampler = None
        try:
            server.start()
            sampler = Sampler(server)
            sampler.thread.start()
            workloads = [("json", "/json"), ("sqlite", "/sqlite"), ("password", "/hash"),
                         ("binary_upload", "/upload"), ("binary_download", "/download"),
                         ("stream", "/stream"), ("paced_reader", "/download"), ("overload", "/slow")]
            for name, route in workloads:
                result = phase(server, sampler, name, route, args.seconds,
                               16 if name == "overload" else 4, args.max_requests,
                               paced=name == "paced_reader")
                report["workloads"].append(result)
                print(name, result["responses"], "responses", flush=True)
            if args.scaling:
                for clients in (1, 8, 32):
                    name = f"json_clients_{clients}"
                    result = phase(server, sampler, name, "/json", args.seconds,
                                   clients, args.max_requests, soak=True)
                    report["workloads"].append(result)
                    print(name, result["responses"], "responses", flush=True)
            stop = threading.Event()
            faults = collections.Counter()
            errors = []
            injector = threading.Thread(target=fault_loop, args=(server, stop, faults, errors), daemon=True)
            injector.start()
            try:
                report["soak"] = phase(server, sampler, "soak", "/json", args.soak_seconds,
                                       4, args.max_requests, soak=True)
            finally:
                stop.set()
                injector.join(timeout=6)
            report["soak"]["faults"] = dict(faults)
            if injector.is_alive() or errors or any(faults[name] == 0 for name in ("/failure", "/timeout", "/abandon")):
                raise RuntimeError("fault injection failed: " + str(errors))
            final = server.recovered()
            if final["workerRecycles"] == 0 or final["workerRestarts"] == 0 or final["workerFailures"] == 0:
                raise RuntimeError("soak did not exercise recycling and failed-worker replacement")
            report["final_metrics"] = final
            report["status"] = "passed"
        except Exception as error:
            report["status"] = "failed"
            report["error"] = type(error).__name__ + ": " + str(error)
        finally:
            if sampler is not None:
                try:
                    sampler.finish()
                    report["samples"] = sampler.samples
                except Exception as error:
                    report["status"] = "failed"
                    report["sampler_error"] = str(error)
            try:
                report["cleanup"] = server.close()
            except Exception as error:
                report["status"] = "failed"
                report["cleanup_error"] = str(error)
        if report["status"] == "passed":
            database = sqlite3.connect(str(directory / "work.sqlite"))
            total = database.execute("SELECT total FROM visits WHERE id=1").fetchone()[0]
            database.close()
            report["sqlite_total"] = total
            expected = next(row["statuses"]["200"] for row in report["workloads"] if row["name"] == "sqlite") + 2
            if total != expected:
                report["status"] = "failed"
                report["error"] = "SQLite persisted counter differs from checked requests plus warmups"
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scripta", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=bounded(1, 30), default=3)
    parser.add_argument("--soak-seconds", type=bounded(10, 300), default=60)
    parser.add_argument("--max-requests", type=bounded(1000, 100000), default=100000)
    parser.add_argument("--scaling", action="store_true",
                        help="also measure JSON at 1, 8, and 32 clients; record 503/504 separately")
    args = parser.parse_args()
    # Reserve output before expensive work; never overwrite an earlier receipt.
    with args.output.open("x", encoding="utf-8") as output:
        report = run(args)
        json.dump(report, output, indent=2, sort_keys=True)
        output.write("\n")
    print(report["status"], args.output, flush=True)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
