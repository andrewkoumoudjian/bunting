#!/usr/bin/env python3
from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path
from dataclasses import dataclass


@dataclass
class JournalResult:
    path: Path
    counts: dict[int, int]
    ok_invariant: bool
    lhs: int
    rhs: int


@dataclass
class OrderResult:
    path: Path
    counts: dict[int, int]
    status4: int


def get_tables(conn: sqlite3.Connection) -> set[str]:
    cur = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    )
    return {row[0] for row in cur.fetchall()}


def is_journal_db(conn: sqlite3.Connection) -> bool:
    tables = get_tables(conn)
    if "journal" not in tables:
        return False

    cur = conn.execute("PRAGMA table_info(journal)")
    cols = {row[1] for row in cur.fetchall()}
    required = {"id", "timestamp", "event_type", "data", "correlation_id"}
    return required.issubset(cols)


def is_order_db(conn: sqlite3.Connection) -> bool:
    tables = get_tables(conn)
    if "orders" not in tables:
        return False

    cur = conn.execute("PRAGMA table_info(orders)")
    cols = {row[1] for row in cur.fetchall()}
    required = {
        "local_id",
        "broker_id",
        "symbol",
        "side",
        "quantity",
        "status",
        "strategy_id",
        "created_at",
        "filled_quantity",
        "avg_fill_price",
        "order_proto",
    }
    return required.issubset(cols)


def query_counts(conn: sqlite3.Connection, table: str, key_col: str) -> dict[int, int]:
    cur = conn.execute(
        f"""
        SELECT {key_col}, COUNT(*) AS n
        FROM {table}
        GROUP BY {key_col}
        ORDER BY {key_col}
        """
    )
    return {int(row[0]): int(row[1]) for row in cur.fetchall()}


def validate_journal(path: Path) -> JournalResult:
    conn = sqlite3.connect(path)
    try:
        counts = query_counts(conn, "journal", "event_type")
        c0 = counts.get(0, 0)
        c2 = counts.get(2, 0)
        c3 = counts.get(3, 0)
        lhs = c2 + c3
        rhs = c0
        ok = lhs == rhs
        return JournalResult(path=path, counts=counts, ok_invariant=ok, lhs=lhs, rhs=rhs)
    finally:
        conn.close()


def validate_orders(path: Path) -> OrderResult:
    conn = sqlite3.connect(path)
    try:
        counts = query_counts(conn, "orders", "status")
        s4 = counts.get(4, 0)
        return OrderResult(path=path, counts=counts, status4=s4)
    finally:
        conn.close()


def find_db_files(root: Path) -> list[Path]:
    return sorted(root.rglob("*.db"))


def pretty_counts(counts: dict[int, int]) -> str:
    if not counts:
        return "{}"
    return ", ".join(f"{k}:{v}" for k, v in sorted(counts.items()))


def pair_results(
    journals: list[JournalResult], orders: list[OrderResult]
) -> list[tuple[JournalResult, OrderResult]]:
    """
    Simple pairing strategy:
    1. If exactly one journal and one order DB exist, pair them.
    2. Otherwise, pair by same parent directory and closest basename.
    """
    if len(journals) == 1 and len(orders) == 1:
        return [(journals[0], orders[0])]

    pairs: list[tuple[JournalResult, OrderResult]] = []
    used_orders: set[Path] = set()

    for j in journals:
        candidates = [
            o for o in orders
            if o.path.parent == j.path.parent and o.path not in used_orders
        ]

        if not candidates:
            continue

        # Prefer basename similarity
        jstem = j.path.stem.lower()
        candidates.sort(
            key=lambda o: (
                0 if any(tok in o.path.stem.lower() for tok in jstem.split("_")) else 1,
                len(o.path.stem),
            )
        )

        chosen = candidates[0]
        pairs.append((j, chosen))
        used_orders.add(chosen.path)

    return pairs


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate quarcc journal/order SQLite DBs")
    parser.add_argument(
        "root",
        nargs="?",
        default=".",
        help="Root directory to scan for .db files (default: current directory)",
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    db_files = find_db_files(root)

    if not db_files:
        print(f"[ERROR] No .db files found under: {root}")
        return 1

    journals: list[JournalResult] = []
    orders: list[OrderResult] = []
    unknown: list[Path] = []

    for db_path in db_files:
        try:
            conn = sqlite3.connect(db_path)
            try:
                if is_journal_db(conn):
                    journals.append(validate_journal(db_path))
                elif is_order_db(conn):
                    orders.append(validate_orders(db_path))
                else:
                    unknown.append(db_path)
            finally:
                conn.close()
        except sqlite3.Error as e:
            print(f"[ERROR] Failed to inspect {db_path}: {e}")
            return 1

    overall_ok = True

    print("=== Journal DB checks ===")
    if not journals:
        print("No journal DBs found.")
        overall_ok = False
    else:
        for j in journals:
            print(f"\n[{j.path}]")
            print(f"event counts: {pretty_counts(j.counts)}")
            print(f"check: event_2 + event_3 == event_0  ->  {j.lhs} == {j.rhs}")
            if j.ok_invariant:
                print("RESULT: PASS")
            else:
                print("RESULT: FAIL")
                overall_ok = False

    print("\n=== Order DB checks ===")
    if not orders:
        print("No order DBs found.")
        overall_ok = False
    else:
        for o in orders:
            print(f"\n[{o.path}]")
            print(f"status counts: {pretty_counts(o.counts)}")
            print(f"status_4 count: {o.status4}")

    print("\n=== Cross DB consistency checks ===")
    pairs = pair_results(journals, orders)

    if not pairs:
        print("No journal/order pairs could be determined automatically.")
        if journals and orders:
            print("Found both journal and order DBs, but pairing is ambiguous.")
        overall_ok = False
    else:
        for j, o in pairs:
            e3 = j.counts.get(3, 0)
            e17 = j.counts.get(17, 0)
            s4 = o.status4

            ok_3 = (s4 == e3)
            ok_17 = (s4 == e17)

            print(f"\nJournal: {j.path}")
            print(f"Orders : {o.path}")
            print(f"check: status_4 == event_3   -> {s4} == {e3}   [{'PASS' if ok_3 else 'FAIL'}]")
            print(f"check: status_4 == event_17  -> {s4} == {e17}  [{'PASS' if ok_17 else 'FAIL'}]")

            if not (ok_3 and ok_17):
                overall_ok = False

    if unknown:
        print("\n=== Unknown .db files skipped ===")
        for p in unknown:
            print(p)

    print("\n=== Final result ===")
    if overall_ok:
        print("ALL CHECKS PASSED")
        return 0
    else:
        print("SOME CHECKS FAILED")
        return 1


if __name__ == "__main__":
    sys.exit(main())
