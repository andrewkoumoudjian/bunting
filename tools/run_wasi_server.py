#!/usr/bin/env python3
"""Run the Bunting WASI server with bounded Wasmer filesystem grants."""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path


def fail(message: str) -> "NoReturn":
    print(message, file=sys.stderr)
    raise SystemExit(2)


script = Path(__file__).resolve()
repo = script.parents[1]
repo_config = repo / "apps/bunting-server/config/local.json"
installed_config = (
    Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    / "bunting/server/local.json"
)
config = Path(
    sys.argv[1]
    if len(sys.argv) > 1
    else os.environ.get(
        "BUNTING_SERVER_CONFIG",
        repo_config if repo_config.is_file() else installed_config,
    )
).resolve()
if not config.is_file():
    fail(f"Bunting server configuration does not exist: {config}")

try:
    document = json.loads(config.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    fail(f"Cannot read Bunting server configuration {config}: {error}")

configured_artifact = os.environ.get("BUNTING_SERVER_ARTIFACT")
candidates = [
    Path(configured_artifact).resolve() if configured_artifact else None,
    repo / "target/wasm32-wasmer-wasi-dl/release/bunting-server.wasmu",
    repo / "target/wasm32-wasmer-wasi-dl/release/bunting-server.wasm",
    script.parent.parent / "share/bunting/bunting-server.wasm",
]
artifact = next((path for path in candidates if path and path.is_file()), None)
if artifact is None:
    fail("Build the WASI server with tools/build_wasi_server.sh before running it")

wasmer = os.environ.get("WASMER_BIN") or shutil.which("wasmer")
if not wasmer:
    fail("Wasmer 7.2.1 is required")

volumes = {config.parent}
for section, key in (("storage", "path"), ("scenario", "path")):
    value = document.get(section)
    path_value = value.get(key) if isinstance(value, dict) else None
    if isinstance(path_value, str):
        resolved = Path(path_value)
        if not resolved.is_absolute():
            resolved = config.parent / resolved
        volumes.add(resolved.resolve().parent)

arguments = [wasmer, "run", str(artifact), "--net"]
for volume in sorted(volumes, key=str):
    arguments.extend(("--volume", f"{volume}:{volume}"))
arguments.extend(("--cwd", str(config.parent), "--", str(config)))
os.execvp(wasmer, arguments)
