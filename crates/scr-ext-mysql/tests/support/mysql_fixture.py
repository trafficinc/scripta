#!/usr/bin/env python3
"""Run one command against a disposable loopback-only, verified-TLS MySQL server."""
import argparse
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
from mysql_commit_proxy import CommitAckProxy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("supply a command after --")
    executables = {name: shutil.which(name) for name in ["mysqld", "mysql", "openssl"]}
    for name, executable in executables.items():
        if executable is None:
            parser.error(name + " must be on PATH")
    temp_parent = "/private/tmp" if Path("/private/tmp").is_dir() else None
    with tempfile.TemporaryDirectory(prefix="scr-mysql-", dir=temp_parent) as directory:
        root = Path(directory)
        data = root / "data"
        data.mkdir()
        log = root / "server.log"
        server = None
        with log.open("wb") as output:
            def run(arguments):
                subprocess.run(arguments, check=True, stdout=output, stderr=output)
            try:
                run([executables["openssl"], "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=Scripta fixture CA", "-addext", "basicConstraints=critical,CA:TRUE", "-keyout", str(root / "ca.key"), "-out", str(root / "ca.crt")])
                run([executables["openssl"], "req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", str(root / "server.key"), "-out", str(root / "server.csr")])
                extensions = root / "extensions.cnf"
                extensions.write_bytes(b"subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
                run([executables["openssl"], "x509", "-req", "-days", "2", "-in", str(root / "server.csr"), "-CA", str(root / "ca.crt"), "-CAkey", str(root / "ca.key"), "-CAcreateserial", "-extfile", str(extensions), "-out", str(root / "server.crt")])
                run([executables["mysqld"], "--no-defaults", "--initialize-insecure", "--datadir=" + str(data)])
                with socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    port = listener.getsockname()[1]
                unix_socket = root / "mysql.sock"
                server_arguments = [executables["mysqld"], "--no-defaults", "--datadir=" + str(data), "--socket=" + str(unix_socket), "--pid-file=" + str(root / "server.pid"), "--port=" + str(port), "--bind-address=127.0.0.1", "--mysqlx=OFF", "--skip-name-resolve", "--require-secure-transport=ON", "--ssl-ca=" + str(root / "ca.crt"), "--ssl-cert=" + str(root / "server.crt"), "--ssl-key=" + str(root / "server.key")]
                sql_mode = os.environ.get("SCR_MYSQL_FIXTURE_SQL_MODE")
                if sql_mode is not None:
                    server_arguments.append("--sql-mode=" + sql_mode)
                server = subprocess.Popen(server_arguments, stdout=output, stderr=output)
                password = "fixture-password-not-for-production"
                sql = "CREATE DATABASE scripta_fixture;"
                for host in ["127.0.0.1", "localhost"]:
                    sql += "CREATE USER 'scripta_fixture'@'" + host + "' IDENTIFIED BY '" + password + "' REQUIRE SSL;"
                    sql += "GRANT ALL ON scripta_fixture.* TO 'scripta_fixture'@'" + host + "';"
                until = time.monotonic() + 30
                while True:
                    if server.poll() is not None:
                        raise RuntimeError("fixture server exited during startup")
                    result = subprocess.run([executables["mysql"], "--no-defaults", "--protocol=socket", "--socket=" + str(unix_socket), "-uroot", "--execute=SELECT 1"], stdout=output, stderr=output)
                    if result.returncode == 0:
                        run([executables["mysql"], "--no-defaults", "--protocol=socket", "--socket=" + str(unix_socket), "-uroot", "--execute=" + sql])
                        break
                    if time.monotonic() >= until:
                        raise RuntimeError("fixture server startup timed out")
                    time.sleep(0.1)
                env = os.environ.copy()
                env.update(SCR_MYSQL_FIXTURE_DSN="mysql:host=localhost;port=" + str(port) + ";dbname=scripta_fixture", SCR_MYSQL_FIXTURE_USER="scripta_fixture", SCR_MYSQL_FIXTURE_PASSWORD=password, SCR_MYSQL_FIXTURE_CA=str(root / "ca.crt"), SCR_MYSQL_FIXTURE_PORT=str(port))
                env.update({f"SCR_DB_{name}": env[f"SCR_MYSQL_FIXTURE_{name}"] for name in ["DSN", "USER", "PASSWORD", "CA"]})
                print("Disposable MySQL TLS fixture ready on loopback", flush=True)
                proxy = CommitAckProxy(root, port)
                try:
                    env["SCR_MYSQL_FIXTURE_COMMIT_PROXY_PORT"] = str(proxy.port)
                    result = subprocess.run(command, env=env)
                    return result.returncode
                finally:
                    proxy.close()
            except Exception:
                output.flush()
                print(log.read_text(errors="replace"))
                raise
            finally:
                if server is not None and server.poll() is None:
                    server.terminate()
                    try:
                        server.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()


if __name__ == "__main__":
    raise SystemExit(main())
