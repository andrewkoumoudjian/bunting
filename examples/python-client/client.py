#!/usr/bin/env python3
"""Minimal dependency-free Bunting FIX Logon and discovery client."""

import socket
from datetime import datetime, timezone

SOH = b"\x01"


def frame(fields):
    body = SOH.join(f"{tag}={value}".encode() for tag, value in fields) + SOH
    head = b"8=FIXT.1.1" + SOH + f"9={len(body)}".encode() + SOH
    partial = head + body
    return partial + f"10={sum(partial) % 256:03}".encode() + SOH


def timestamp():
    return datetime.now(timezone.utc).strftime("%Y%m%d-%H:%M:%S")


with socket.create_connection(("127.0.0.1", 9880), timeout=5) as connection:
    connection.sendall(
        frame(
            [
                (35, "A"),
                (49, "HUMAN"),
                (56, "BUNTING"),
                (34, 1),
                (52, timestamp()),
                (98, 0),
                (108, 30),
                (1137, 9),
                (553, "participant"),
                (554, "bunting-local-dev"),
                (10000, "bunting.fixlatest.competition.v1"),
                (10004, "participant"),
            ]
        )
    )
    print(connection.recv(65536).replace(SOH, b"|").decode())
