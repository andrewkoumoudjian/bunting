#!/usr/bin/env python3
"""Drive a running Bunting server with crossing FIX orders and time each ack.

Used by tools/host_parity.sh to give native and WASIX servers the same order
flow (ADR 0044). Two roster participants log on; each buys then sells through
the agents' quotes, so the run produces resting orders, fills and
trades. One order is in flight at a time: the figure reported is the
order-to-first-ExecutionReport round trip seen by a loopback client.

usage: host_load.py <host:port> <orders> [--json]
"""

from __future__ import annotations

import json
import socket
import statistics
import sys
import time
from datetime import datetime, timezone

SOH = b"\x01"
SENDER = "BUNTING"
TEAMS = (
    ("HUMAN", "participant", "bunting-local-dev"),
    ("TEAM2", "team2", "bunting-team2-dev"),
)


def frame(fields: list[tuple[int, object]]) -> bytes:
    body = SOH.join(f"{tag}={value}".encode() for tag, value in fields) + SOH
    head = b"8=FIXT.1.1" + SOH + f"9={len(body)}".encode() + SOH
    partial = head + body
    return partial + f"10={sum(partial) % 256:03}".encode() + SOH


def timestamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%d-%H:%M:%S.%f")[:-3]


class Session:
    def __init__(self, endpoint: tuple[str, int], comp_id: str, user: str, password: str):
        self.comp_id = comp_id
        self.sequence = 0
        self.buffer = b""
        self.socket = socket.create_connection(endpoint, timeout=10)
        self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.send(
            "A",
            [
                (98, 0),
                (108, 30),
                (1137, 9),
                (553, user),
                (554, password),
                (10000, "bunting.fixlatest.competition.v1"),
                (10004, "participant"),
            ],
        )
        logon = self.read_until(lambda fields: fields.get("35") == "A")
        if logon is None:
            raise SystemExit(f"{comp_id}: logon was not answered")

    def send(self, message_type: str, body: list[tuple[int, object]]) -> None:
        self.sequence += 1
        header = [
            (35, message_type),
            (49, self.comp_id),
            (56, SENDER),
            (34, self.sequence),
            (52, timestamp()),
        ]
        self.socket.sendall(frame(header + body))

    def next_message(self) -> dict[str, str]:
        while True:
            end = self.buffer.find(SOH + b"10=")
            if end >= 0:
                close = self.buffer.find(SOH, end + 1)
                if close >= 0:
                    raw, self.buffer = self.buffer[: close + 1], self.buffer[close + 1 :]
                    fields = {}
                    for item in raw.split(SOH):
                        if b"=" in item:
                            tag, value = item.split(b"=", 1)
                            fields.setdefault(tag.decode(), value.decode())
                    return fields
            chunk = self.socket.recv(65536)
            if not chunk:
                raise SystemExit(f"{self.comp_id}: server closed the connection")
            self.buffer += chunk

    def read_until(self, predicate):
        for _ in range(10_000):
            fields = self.next_message()
            if fields.get("35") in ("3", "5", "j"):
                raise SystemExit(f"{self.comp_id}: {fields}")
            if fields.get("35") == "1":
                # The venue probes round-trip time with TestRequests.
                self.send("0", [(112, fields.get("112", ""))])
                continue
            if predicate(fields):
                return fields
        return None


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    host, port = sys.argv[1].rsplit(":", 1)
    orders = int(sys.argv[2])
    sessions = [Session((host, int(port)), *team) for team in TEAMS]
    samples = []
    rejected = 0
    for index in range(orders):
        session = sessions[index % 2]
        client_order_id = str(index + 1)
        # Each team buys, then sells, at prices through the agents' quotes,
        # so orders trade against the agents and each other. Sells without
        # inventory are rejected; rejections are journaled inputs too.
        side = "1" if index % 4 < 2 else "2"
        price = 102 if side == "1" else 98
        started = time.perf_counter_ns()
        session.send(
            "D",
            [
                (11, client_order_id),
                (48, 1),
                (207, 1),
                (54, side),
                (38, 1 + index % 3),
                (40, 2),
                (44, price),
            ],
        )
        report = session.read_until(
            lambda fields: fields.get("35") == "8" and fields.get("11") == client_order_id
        )
        samples.append(time.perf_counter_ns() - started)
        if report is None:
            raise SystemExit(f"order {client_order_id} was not answered")
        rejected += report.get("39") == "8"
    samples.sort()
    summary = {
        "orders": orders,
        "rejected": rejected,
        "p50_us": round(samples[len(samples) // 2] / 1000, 1),
        "p99_us": round(samples[min(len(samples) - 1, len(samples) * 99 // 100)] / 1000, 1),
        "mean_us": round(statistics.fmean(samples) / 1000, 1),
        "max_us": round(samples[-1] / 1000, 1),
    }
    print(json.dumps(summary) if "--json" in sys.argv else summary)


if __name__ == "__main__":
    main()
