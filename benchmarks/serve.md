# Local HTTP measurement protocol

Define the following before measuring; do not run concurrent builds/tests during
measurement. The Python standard-library driver `serve.py` owns temporary source,
SQLite/data/spool fixtures and loopback listeners; it uses no public HTTP endpoints
or external service manager. Build an optimized executable with SQLite first:

```sh
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p scr-cli --profile dist --locked --offline --target aarch64-apple-darwin
python3 benchmarks/serve.py --scripta target/aarch64-apple-darwin/dist/scripta --output /private/tmp/scripta-serve-measurement.json
```

Use your native explicit target on other hosts. The deployment setting alone does
not establish macOS 15 runtime support. Reports include revision/dirty paths,
compiler/host/architecture, executable/fixture/driver hashes, exact parameters,
fixed server counters, observed RSS, and cleanup results. New output paths are
required; preserve earlier receipts. `--seconds` defaults to 3 per workload
(bounds 1–30); `--soak-seconds` defaults to 60 (bounds 10–300). Each phase also has a
100000-request ceiling; reports state elapsed duration and whether that ceiling
ended it. A short `--seconds 1 --soak-seconds 10` run is a functional exercise,
not the default sustained-load observation.

Add `--scaling` to measure the same checked JSON endpoint with 1, 8, and 32
concurrent clients, alongside the normal four-client phase. Worker count and
queue settings remain fixed. These extra phases permit and count 503/504
responses separately, report successful throughput and latency, and require
full recovery after each phase. Increasing client count can expose saturation;
this shared-host Python driver does not establish maximum server capacity.

Server settings: two worker processes, queue eight, at most 64 connections,
750 ms request deadline, 120 s connection lifetime, 1500 ms shutdown drain,
request-count recycling at 200 completed exchanges, JSON stderr logs, private
loopback health listener, ordinary production mode (no watch). SQLite uses a
local pre-created table, one atomic UPDATE and SELECT per request, 500 ms busy
wait, default journal/durability; the script does not benchmark transaction batches.
Two warmups per route precede timing. Four Python threads each reuse an HTTP/1.1
connection. There is no automatic request retry on transport failure.

| Workload | Validated output / controlled cost |
| --- | --- |
| Small JSON | Fresh runtime, request lookup, encode/decode `{ok:true,path:"/json"}`. |
| SQLite | Open local DB, UPDATE, SELECT positive counter, close. |
| Password | Hash and verify public fixture input using defaults: Argon2id 19456 KiB, two iterations, parallelism one. |
| Binary upload | POST all 256 byte values repeated 1024 times (256 KiB); exact echoed bytes. |
| Binary download | Stream/download the same 256 KiB local file; exact bytes. |
| Streaming | Four flushed `chunk-N` lines, 5 ms sleep after each; exact assembled body and time to first body byte. |
| Paced reader | Same download, 4096-byte reads with 2 ms pauses; includes intentional client delay. |
| Overload | 16 clients against 100 ms sleeps; 200/503 are expected, with success latency separated from fast rejections. |

After each phase, probe recovery, wait for zero request/queue/busy gauges, and
verify no owned spools remain. Counter snapshots and status distributions expose
worker recycling, readiness changes, overload, and failure recovery. Throughput
is checked completed responses divided by measured wall time; successful-response
throughput is reported separately. Nearest-rank p50/p95/p99/min/max are calculated
from bounded observations; response completion means the Python client consumed
the checked body. TTFB includes request send, headers, and first body byte, not a
server-only timestamp. Percentiles over tiny samples are descriptive only.

The default 60-second soak runs the JSON load while a fifth thread cycles through
buffered application errors (500), execution timeouts (504), and disconnecting an
8 MiB flushed response after the first byte, carrying an owned raw upload spool.
Faults are spaced at least 0.5 seconds apart. Normal soak load may observe 503/504
under recovery pressure and records them separately; other statuses, incorrect
successful bodies, transport errors, absent faults/recycling/restarts, unrecovered
capacity, residual spools, or surviving worker PIDs fail the driver. This is a
bounded soak, not proof of absence of leaks or a production availability claim.

A sampler uses `ps` on macOS/Linux every 0.5 seconds for the supervisor and its
direct worker children, and private metrics at the same interval. RSS values are
observed resident bytes, not heap allocations, unique memory, OS high-water marks, or
logical Scripta value budgets. Summing process RSS can double-count shared pages;
allocator retention and worker recycling affect trends. Sampling and Python/GIL
client cost share the host with the server. Windows RSS is unavailable in this
stdlib-only driver; correctness still runs, but its console shutdown/native
behavior requires that host's evidence. macOS/Linux are the measurement targets
for this initial driver. No cross-language, public-TLS, proxy, throughput parity,
CPU-profile, cold-cache, or cross-platform speed claim is implied. Optimize only
when a measured bottleneck and meaningful regression justify a change.
