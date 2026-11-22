"""Controlled local inputs and independent expected results for the extended suite."""
from contextlib import contextmanager
import csv
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
from pathlib import Path
import threading

CASES = ("startup", "numeric_float", "numeric_nested", "numeric_bounds", "numeric_calls",
         "files", "csv_json", "subprocess", "http_stream", "sqlite")
PAYLOAD = bytes(range(256)) * 256
CSV_NAME = 'comma, quote" and\nline é'


def modulo_sum(n):
    cycles, tail = divmod(n, 97)
    return cycles * 97 * 96 // 2 + tail * (tail - 1) // 2


def bounds_iterations(iterations):
    completed = 0
    while completed < iterations - (completed + 10) // 11:
        completed += 1
    return completed


def expected(case, iterations):
    if case == "numeric_float":
        return modulo_sum(iterations)
    if case == "numeric_nested":
        width = max(1, iterations // 20)
        return sum(modulo_sum(width + offset) - modulo_sum(offset) for offset in range(20))
    if case == "numeric_bounds":
        completed = bounds_iterations(iterations)
        return completed * (completed - 1) // 2
    if case == "numeric_calls":
        return 3 * modulo_sum(iterations) + iterations
    return {"files": 512 * 100000 + 6144, "csv_json": 38760,
            "subprocess": 27, "http_stream": len(PAYLOAD), "sqlite": 2562040}[case]


def units(case, iterations):
    return {"startup": (1, "launches"), "numeric_float": (iterations, "iterations"),
            "numeric_nested": (20 * max(1, iterations // 20), "inner iterations"),
            "numeric_bounds": (bounds_iterations(iterations), "iterations"),
            "numeric_calls": (iterations, "calls"), "files": (512, "lines"),
            "csv_json": (256, "rows"), "subprocess": (3, "children"),
            "http_stream": (len(PAYLOAD), "bytes"), "sqlite": (256, "inserted rows")}[case]


def prepare(directory):
    directory = Path(directory)
    text = "".join(f"line-{i:04d}:7\n" for i in range(512))
    (directory / "input.txt").write_bytes(text.encode("ascii"))
    output = io.StringIO(newline="")
    writer = csv.writer(output, lineterminator="\r\n")
    writer.writerows((i, CSV_NAME, i % 17) for i in range(256))
    (directory / "input.csv").write_bytes(output.getvalue().encode("utf-8"))
    (directory / "payload.bin").write_bytes(PAYLOAD)
    (directory / "child.py").write_bytes((Path(__file__).parent / "fixtures" / "child.py").read_bytes())
    return text.upper().encode("ascii")


@contextmanager
def local_http(enabled):
    if not enabled:
        yield "", None
        return

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def setup(self):
            super().setup()
            self.connection.settimeout(5)

        def do_GET(self):
            if self.path != "/stream":
                self.send_error(404)
                return
            self.connection.settimeout(5)
            with self.server.count_lock:
                self.server.requests += 1
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            for offset in range(0, len(PAYLOAD), 1024):
                chunk = PAYLOAD[offset:offset + 1024]
                self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
            self.close_connection = True

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    server.requests = 0
    server.count_lock = threading.Lock()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/stream", server
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        if thread.is_alive():
            raise RuntimeError("local HTTP server did not stop")
