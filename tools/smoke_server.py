#!/usr/bin/env python3
"""Smoke-test a Bunting venue server on any host (ADR 0044).

Writes a loopback configuration with the durable file origin and built-in
agents into <state-dir>, starts the server, waits for /health, trades a few
orders over FIX (tools/host_load.py), stops it, restarts it on the same
journal and requires the run to come back at the same committed sequence.

The server command is everything after `--`; the configuration path is
appended to it. Examples:

  tools/smoke_server.py --state-dir /tmp/s -- target/release/bunting server
  tools/smoke_server.py --state-dir /tmp/s --config-path /etc/bunting/server.json \\
      --storage-path /var/lib/bunting/origin.json -- \\
      docker run --rm --network host -v /tmp/s:/etc/bunting:ro bunting:dev server

`--config-path` and `--storage-path` name the paths as the server sees them
(inside a container); they default to files in <state-dir>.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import time
import urllib.request

REPO = pathlib.Path(__file__).resolve().parents[1]
ADMIN = "http://127.0.0.1:8080"
TOKEN = "replace-admin-token"


def admin(path: str) -> dict:
    request = urllib.request.Request(ADMIN + path, headers={"Authorization": f"Bearer {TOKEN}"})
    with urllib.request.urlopen(request, timeout=2) as response:
        return json.load(response)


def wait_healthy(process: subprocess.Popen) -> None:
    for _ in range(240):
        if process.poll() is not None:
            raise SystemExit(f"server exited with {process.returncode} before becoming healthy")
        try:
            with urllib.request.urlopen(ADMIN + "/health", timeout=1) as response:
                if json.load(response).get("status") == "ok":
                    return
        except OSError:
            pass
        time.sleep(0.25)
    raise SystemExit("server did not become healthy")


def start(command: list[str], log: pathlib.Path) -> subprocess.Popen:
    handle = log.open("ab")
    process = subprocess.Popen(command, stdout=handle, stderr=subprocess.STDOUT)
    try:
        wait_healthy(process)
    except SystemExit:
        stop(process)
        sys.stderr.write(log.read_text(errors="replace"))
        raise
    return process


def stop(process: subprocess.Popen) -> None:
    process.terminate()
    try:
        process.wait(timeout=20)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--state-dir", required=True, type=pathlib.Path)
    parser.add_argument("--config-path")
    parser.add_argument("--storage-path")
    parser.add_argument("--orders", type=int, default=40)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    arguments = parser.parse_args()
    command = arguments.command[1:] if arguments.command[:1] == ["--"] else arguments.command
    if not command:
        parser.error("give the server command after --")

    state = arguments.state_dir.resolve()
    state.mkdir(parents=True, exist_ok=True)
    config = json.loads((REPO / "apps/bunting-server/config/local.json").read_text())
    config["storage"]["path"] = arguments.storage_path or str(state / "origin.json")
    config_path = arguments.config_path or str(state / "server.json")
    config_dir = pathlib.PurePath(config_path).parent
    config["scenario"]["path"] = str(config_dir / "scenario.json")
    (state / "server.json").write_text(json.dumps(config))
    (state / "scenario.json").write_bytes(
        (REPO / "apps/bunting-server/config/scenario.json").read_bytes()
    )
    log = state / "server.log"

    process = start([*command, config_path], log)
    try:
        subprocess.run(
            [sys.executable, str(REPO / "tools/host_load.py"), "127.0.0.1:9880", str(arguments.orders)],
            check=True,
        )
        before = admin("/admin/runs/1")
    finally:
        stop(process)
    print(f"before restart: {before}")
    if int(before["committedSequence"]) < arguments.orders:
        raise SystemExit("fewer commands committed than orders sent")

    process = start([*command, config_path], log)
    try:
        after = admin("/admin/runs/1")
    finally:
        stop(process)
    print(f"after restart:  {after}")
    # Built-in agents may commit before the first read after restart, never fewer.
    if int(after["committedSequence"]) < int(before["committedSequence"]):
        raise SystemExit("restart lost committed commands")
    print("smoke: ok")


if __name__ == "__main__":
    main()
