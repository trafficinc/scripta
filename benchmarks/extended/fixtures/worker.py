"""Extended benchmark worker; arguments are supplied by ../run.py."""
import json
import sys
import time


def mix(value):
    return (value % 97) * 3 + 1


def work(case, n, directory, url, python):
    if case == "numeric_float":
        total = 0.0
        for i in range(n):
            total += float(i % 97) * 0.5
        return int(total * 2.0)
    if case == "numeric_nested":
        total = 0
        width = max(1, n // 20)
        for outer in range(20):
            for i in range(width):
                total += (outer + i) % 97
        return total
    if case == "numeric_bounds":
        total, i, bound = 0, 0, n
        while i < bound:
            if i % 11 == 0:
                bound -= 1
            total += i
            i += 1
        return total
    if case == "numeric_calls":
        total = 0
        for i in range(n):
            total += mix(i)
        return total
    if case == "files":
        text, count = "", 0
        with (directory / "input.txt").open(encoding="utf-8", newline="") as source:
            for line in source:
                text += line.rstrip("\r\n").upper() + "\n"
                count += 1
        target = directory / "output-python.txt"
        target.write_bytes(text.encode("utf-8"))
        return count * 100000 + len(target.read_bytes())
    if case == "csv_json":
        with (directory / "input.csv").open(encoding="utf-8", newline="") as source:
            rows = list(csv.reader(source))
        if rows[0][1] != 'comma, quote" and\nline é':
            raise ValueError("CSV string mismatch")
        values = [{"id": int(r[0]), "name": r[1], "value": int(r[2]) * 3} for r in rows]
        decoded = json.loads(json.dumps(values, ensure_ascii=False, separators=(",", ":")))
        if decoded != values:
            raise ValueError("JSON roundtrip mismatch")
        return sum(row["id"] + row["value"] for row in decoded)
    if case == "subprocess":
        total = 0
        for _ in range(3):
            result = subprocess.run([python, "-I", str(directory / "child.py")],
                                    capture_output=True, check=True, timeout=5)
            if result.stdout != b"child-ok\n" or result.stderr:
                raise ValueError("child output mismatch")
            total += len(result.stdout)
        return total
    if case == "http_stream":
        expected = (directory / "payload.bin").read_bytes()
        offset = 0
        with urllib.request.urlopen(url, timeout=5) as response:
            if response.status != 200:
                raise ValueError("HTTP status mismatch")
            while True:
                chunk = response.read(4096)
                if not chunk:
                    break
                if chunk != expected[offset:offset + len(chunk)]:
                    raise ValueError("HTTP bytes mismatch")
                offset += len(chunk)
        return offset
    if case == "sqlite":
        db = sqlite3.connect(":memory:", cached_statements=0, isolation_level=None)
        try:
            db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)")
            db.execute("BEGIN")
            for i in range(256):
                db.execute("INSERT INTO items VALUES (?, ?)", (i, i % 17))
            db.execute("COMMIT")
            count, total = db.execute("SELECT COUNT(*) AS count, SUM(value) AS total FROM items").fetchone()
            return count * 10000 + total
        finally:
            db.close()
    raise ValueError("unknown workload")


if __name__ == "__main__":
    case, n, batches, directory, url, python = sys.argv[1:]
    n, batches = int(n), int(batches)
    # Load workload dependencies before timing, without charging unrelated
    # libraries to numeric workers' peak RSS.
    if case in ("files", "csv_json", "subprocess", "http_stream"):
        from pathlib import Path
        directory = Path(directory)
    if case == "csv_json":
        import csv
    if case == "subprocess":
        import subprocess
    if case == "http_stream":
        import urllib.request
    if case == "sqlite":
        import sqlite3
    times, checksums = [], []
    for _ in range(batches):
        start = time.perf_counter_ns()
        checksum = work(case, n, directory, url, python)
        times.append((time.perf_counter_ns() - start) / 1_000_000)
        checksums.append(checksum)
    print(json.dumps({"case": case, "elapsed_ms": times, "checksums": checksums}))
