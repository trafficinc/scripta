#!/usr/bin/env python3
"""Run a command against an isolated loopback PostgreSQL SCRAM/TLS fixture.

Requires initdb, pg_ctl, psql and openssl on PATH. Never starts a system service.
"""
import argparse
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
from postgres_commit_proxy import CommitAckProxy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("supply a command after --")
    tools = {name: shutil.which(name) for name in ["initdb", "pg_ctl", "psql", "openssl"]}
    for name, tool in tools.items():
        if tool is None:
            parser.error(name + " must be on PATH")
    parent = "/private/tmp" if Path("/private/tmp").is_dir() else None
    with tempfile.TemporaryDirectory(prefix="scr-pg-", dir=parent) as directory:
        root = Path(directory)
        data = root / "data"
        password = "fixture-password-not-for-production"
        password_file = root / "password"
        password_file.write_text(password + "\n", encoding="utf-8")
        password_file.chmod(0o600)
        started = False
        with (root / "setup.log").open("wb") as output:
            def run(arguments, **kwargs):
                return subprocess.run(arguments, check=True, stdout=output, stderr=output, **kwargs)
            try:
                run([tools["initdb"], "-D", str(data), "-U", "scripta_fixture", "--pwfile=" + str(password_file), "--auth-local=trust", "--auth-host=scram-sha-256", "--encoding=UTF8", "--no-locale"])
                run([tools["openssl"], "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=Scripta PostgreSQL fixture CA", "-addext", "basicConstraints=critical,CA:TRUE", "-keyout", str(root / "ca.key"), "-out", str(root / "ca.crt")])
                run([tools["openssl"], "req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost", "-keyout", str(data / "server.key"), "-out", str(root / "server.csr")])
                (data / "server.key").chmod(0o600)
                (root / "extensions.cnf").write_bytes(b"subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
                run([tools["openssl"], "x509", "-req", "-days", "2", "-in", str(root / "server.csr"), "-CA", str(root / "ca.crt"), "-CAkey", str(root / "ca.key"), "-CAcreateserial", "-extfile", str(root / "extensions.cnf"), "-out", str(data / "server.crt")])
                with socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    port = listener.getsockname()[1]
                config = f"\nlisten_addresses='127.0.0.1'\nport={port}\nssl=on\npassword_encryption='scram-sha-256'\nunix_socket_directories='{root.as_posix()}'\nfsync=off\n"
                with (data / "postgresql.conf").open("a", encoding="utf-8") as file:
                    file.write(config)
                (data / "pg_hba.conf").write_bytes(b"local all all trust\nhostssl all all 127.0.0.1/32 scram-sha-256\nhost all all 0.0.0.0/0 reject\n")
                run([tools["pg_ctl"], "-D", str(data), "-l", str(root / "server.log"), "-w", "start"])
                started = True
                psql = [tools["psql"], "-X", "-v", "ON_ERROR_STOP=1", "-h", str(root), "-p", str(port), "-U", "scripta_fixture", "-d", "postgres"]
                run(psql + ["-c", "CREATE DATABASE scripta_fixture"])
                # Administrative setup enables trigger/sequence regressions while
                # the public API deliberately rejects function and sequence DDL.
                setup = """
CREATE SEQUENCE trigger_sequence START 1000;
CREATE TABLE sequence_items(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, label text UNIQUE, audit_id bigint);
CREATE FUNCTION assign_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.audit_id := nextval('trigger_sequence'); RETURN NEW; END $$;
CREATE TRIGGER sequence_audit BEFORE INSERT ON sequence_items FOR EACH ROW EXECUTE FUNCTION assign_audit();
"""
                run(psql[:-1] + ["scripta_fixture", "-c", setup])
                env = os.environ.copy()
                env.update(SCR_PG_FIXTURE_DSN=f"pgsql:host=localhost;port={port};dbname=scripta_fixture", SCR_PG_FIXTURE_USER="scripta_fixture", SCR_PG_FIXTURE_PASSWORD=password, SCR_PG_FIXTURE_CA=str(root / "ca.crt"), SCR_PG_FIXTURE_PORT=str(port))
                env.update({f"SCR_DB_{name}": env[f"SCR_PG_FIXTURE_{name}"] for name in ["DSN", "USER", "PASSWORD", "CA"]})
                print("Disposable PostgreSQL SCRAM/TLS fixture ready on loopback", flush=True)
                proxy = CommitAckProxy(root, port)
                try:
                    env["SCR_PG_FIXTURE_COMMIT_PROXY_PORT"] = str(proxy.port)
                    return subprocess.run(command, env=env).returncode
                finally:
                    proxy.close()
            except Exception:
                output.flush()
                print((root / "setup.log").read_text(errors="replace"))
                if (root / "server.log").exists():
                    print((root / "server.log").read_text(errors="replace"))
                raise
            finally:
                if started:
                    run([tools["pg_ctl"], "-D", str(data), "-m", "immediate", "-w", "stop"])


if __name__ == "__main__":
    raise SystemExit(main())
