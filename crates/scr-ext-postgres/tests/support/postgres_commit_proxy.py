"""Local fixture proxy that suppresses successful COMMIT acknowledgements.

TLS terminates on both sides using only the disposable fixture certificates.
Application packets are forwarded without logging or retaining credentials/SQL.
"""
import socket
import ssl
import threading


def exact(stream, count):
    data = bytearray()
    while len(data) < count:
        part = stream.recv(count - len(data))
        if not part:
            raise EOFError()
        data.extend(part)
    return bytes(data)


def packet(stream, typed=True):
    header = exact(stream, 5 if typed else 4)
    size = int.from_bytes(header[1:] if typed else header, "big")
    if size < 4 or size > 1024 * 1024:
        raise ValueError("fixture proxy packet ceiling exceeded")
    return header + exact(stream, size - 4)


class CommitAckProxy:
    def __init__(self, root, backend_port):
        self.backend_port = backend_port
        self.server_tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.server_tls.load_cert_chain(root / "data/server.crt", root / "data/server.key")
        self.client_tls = ssl.create_default_context(cafile=str(root / "ca.crt"))
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.stopped = threading.Event()
        self.sockets = []
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    def accept(self):
        while not self.stopped.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            threading.Thread(target=self.session, args=(client,), daemon=True).start()

    def session(self, client):
        upstream = None
        streams = []
        try:
            client.settimeout(5)
            upstream = socket.create_connection(("127.0.0.1", self.backend_port), timeout=5)
            request = packet(client, typed=False)
            if request != bytes.fromhex("0000000804d2162f"):
                raise ValueError("expected PostgreSQL SSLRequest")
            upstream.sendall(request)
            response = exact(upstream, 1)
            client.sendall(response)
            if response != b"S":
                raise ValueError("TLS required")
            upstream = self.client_tls.wrap_socket(upstream, server_hostname="localhost")
            client = self.server_tls.wrap_socket(client, server_side=True)
            upstream.sendall(packet(client, typed=False))
            streams = [client, upstream]
            with self.lock:
                self.sockets.extend(streams)
            commit = threading.Event()

            def responses():
                try:
                    while not self.stopped.is_set():
                        response = packet(upstream)
                        if commit.is_set() and response[:1] == b"C" and response[5:] == b"COMMIT\0":
                            # The real server has returned success. Drop only the
                            # acknowledgement; do not resend any command.
                            break
                        client.sendall(response)
                except (OSError, EOFError, ValueError):
                    pass
                finally:
                    for stream in streams:
                        try:
                            stream.shutdown(socket.SHUT_RDWR)
                        except OSError:
                            pass
                        stream.close()

            reader = threading.Thread(target=responses, daemon=True)
            reader.start()
            while not self.stopped.is_set():
                request = packet(client)
                if request[:1] == b"Q" and request[5:].rstrip(b"\0").strip().upper() == b"COMMIT":
                    commit.set()
                upstream.sendall(request)
        except (OSError, EOFError, ValueError):
            pass
        finally:
            for stream in [client, upstream]:
                if stream is not None:
                    stream.close()
            with self.lock:
                for stream in streams:
                    if stream in self.sockets:
                        self.sockets.remove(stream)

    def close(self):
        self.stopped.set()
        self.listener.close()
        with self.lock:
            for stream in self.sockets:
                try:
                    stream.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                stream.close()
        self.thread.join(timeout=1)
