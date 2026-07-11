#!/usr/bin/env python3
"""quarcc trading engine — unified test client.

Usage examples:
  # Run a single strategy (name or 1-based index)
  python3 client.py run 1
  python3 client.py run simple_test_strategy_2 --iterations 50

  # Run all strategies sequentially or in parallel
  python3 client.py run-all
  python3 client.py run-all --parallel

  # Query positions
  python3 client.py positions

  # Kill switch
  python3 client.py kill --reason "done testing" --by dev

  # Print config with gateway overridden (pipe to engine)
  python3 client.py gen-config --gateway alpaca > /tmp/quarcc.yaml
  ./build/engine-cpp/src/trading_engine /tmp/quarcc.yaml
"""

import logging

# Set WARNING *before* importing grpc_interface so its logging.basicConfig() is a no-op.
logging.basicConfig(level=logging.WARNING)

import argparse
import os
import sys
import threading
import time

import yaml

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)               # for grpc_interface
sys.path.insert(0, os.path.dirname(_HERE))  # for gen.python.contracts

import grpc_interface

# Defaults (used when config.yaml has no `test` section for a strategy)
DEFAULT_CONFIG = os.path.join(_HERE, "config.yaml")
DEFAULT_SERVER = "localhost:50051"
DEFAULT_ORDERS = [
    {"side": "BUY",  "qty": 2.0},
    {"side": "SELL", "qty": 1.5},
]
DEFAULT_ITERATIONS = 30
DEFAULT_WAIT = 3.0

# Lock so parallel strategy output lines don't interleave mid-line.
_PRINT_LOCK = threading.Lock()

def _print(*args_, **kwargs):
    with _PRINT_LOCK:
        print(*args_, **kwargs)


# Config helpers
def load_config(path: str) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def resolve_strategy(config: dict, identifier: str) -> dict | None:
    """Resolve by exact ID string or 1-based numeric index."""
    strategies = config.get("strategies", [])
    for s in strategies:
        if s["id"] == identifier:
            return s
    try:
        idx = int(identifier) - 1
        if 0 <= idx < len(strategies):
            return strategies[idx]
    except ValueError:
        pass
    return None


# Core run logic
def run_strategy(
    client: grpc_interface.ExecutionClient,
    strategy: dict,
    *,
    iterations: int | None = None,
    wait: float | None = None,
    flatten: bool | None = None,
    verbose: bool = False,
) -> None:
    if verbose:
        logging.getLogger("grpc_interface").setLevel(logging.INFO)

    strat_id  = strategy["id"]
    test      = strategy.get("test", {})
    symbol    = test.get("symbol", "ACDC")
    n_iters   = iterations if iterations is not None else test.get("iterations",  DEFAULT_ITERATIONS)
    wait_secs = wait       if wait       is not None else test.get("wait_secs",   DEFAULT_WAIT)
    orders    = test.get("orders", DEFAULT_ORDERS)
    do_flatten = flatten if flatten is not None else test.get("flatten", True)

    _print(f"[{strat_id}] {symbol}  {n_iters} iters × {len(orders)} orders")

    t0 = time.perf_counter()
    for _ in range(n_iters):
        for o in orders:
            client.submit_signal(strat_id, symbol, o["side"], float(o["qty"]))
    elapsed = time.perf_counter() - t0

    _print(f"[{strat_id}] submitted {n_iters * len(orders)} signals in {elapsed:.3f}s")

    if do_flatten:
        _print(f"[{strat_id}] waiting {wait_secs}s for fills…")
        time.sleep(wait_secs)

        pos = client.get_position(symbol)
        if pos and pos["quantity"] != 0.0:
            qty  = pos["quantity"]
            side = "SELL" if qty > 0 else "BUY"
            _print(f"[{strat_id}] flattening: {side} {abs(qty):.4f} {symbol}")
            client.submit_signal(strat_id, symbol, side, abs(qty))
            time.sleep(wait_secs)

    pos = client.get_position(symbol)
    if pos:
        sign = "+" if pos["realized_pnl"] >= 0 else ""
        _print(
            f"[{strat_id}] {symbol}  "
            f"qty={pos['quantity']:+.4f}  "
            f"avg={pos['avg_price']:.4f}  "
            f"rPnL={sign}{pos['realized_pnl']:.4f}"
        )


# Sub-commands
def cmd_run(args) -> None:
    config   = load_config(args.config)
    strategy = resolve_strategy(config, args.strategy)
    if strategy is None:
        _die(f"strategy '{args.strategy}' not found in {args.config}")

    _check_server(args.server)
    client = grpc_interface.ExecutionClient(args.server)
    try:
        run_strategy(
            client, strategy,
            iterations=args.iterations,
            wait=args.wait,
            flatten=None if not args.no_flatten else False,
            verbose=args.verbose,
        )
    finally:
        client.close()


def cmd_run_all(args) -> None:
    config     = load_config(args.config)
    strategies = config.get("strategies", [])
    if not strategies:
        _die(f"no strategies in {args.config}")

    _check_server(args.server)
    flatten = None if not args.no_flatten else False

    if args.parallel:
        errors: list[str] = []
        err_lock = threading.Lock()

        def _run_one(strat: dict) -> None:
            c = grpc_interface.ExecutionClient(args.server)
            try:
                run_strategy(
                    c, strat,
                    iterations=args.iterations,
                    wait=args.wait,
                    flatten=flatten,
                    verbose=args.verbose,
                )
            except Exception as exc:  # noqa: BLE001
                with err_lock:
                    errors.append(f"[{strat['id']}] {exc}")
            finally:
                c.close()

        threads = [
            threading.Thread(target=_run_one, args=(s,), daemon=True)
            for s in strategies
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        if errors:
            for e in errors:
                print(e, file=sys.stderr)
            sys.exit(1)
    else:
        client = grpc_interface.ExecutionClient(args.server)
        try:
            for strat in strategies:
                run_strategy(
                    client, strat,
                    iterations=args.iterations,
                    wait=args.wait,
                    flatten=flatten,
                    verbose=args.verbose,
                )
        finally:
            client.close()


def cmd_positions(args) -> None:
    _check_server(args.server)
    client = grpc_interface.ExecutionClient(args.server)
    try:
        positions = client.get_all_positions()
        if not positions:
            print("No open positions.")
            return
        hdr = f"{'SYMBOL':<10} {'QTY':>12} {'AVG PRICE':>12} {'rPnL':>12}"
        print(hdr)
        print("-" * len(hdr))
        for p in sorted(positions, key=lambda x: x["symbol"]):
            sign = "+" if p["realized_pnl"] >= 0 else ""
            print(
                f"{p['symbol']:<10} {p['quantity']:>+12.4f}"
                f" {p['avg_price']:>12.4f}"
                f" {sign}{p['realized_pnl']:>11.4f}"
            )
    finally:
        client.close()


def cmd_kill(args) -> None:
    _check_server(args.server)
    client = grpc_interface.ExecutionClient(args.server)
    try:
        client.activate_kill_switch(args.reason, args.initiated_by)
        print(f"Kill switch sent: {args.reason}")
    finally:
        client.close()


def cmd_gen_config(args) -> None:
    """Print a (possibly gateway-patched) config.yaml to stdout."""
    config = load_config(args.config)
    if args.gateway:
        for strat in config.get("strategies", []):
            strat["gateway"] = args.gateway
    # Use sort_keys=False to preserve insertion order (Python 3.7+).
    print(yaml.dump(config, default_flow_style=False, sort_keys=False, allow_unicode=True), end="")


# Helpers
def _check_server(address: str) -> None:
    """Exit with a clear message if the engine is not reachable."""
    import grpc as _grpc
    ch = _grpc.insecure_channel(address)
    try:
        _grpc.channel_ready_future(ch).result(timeout=2.0)
    except Exception:
        ch.close()
        _die(f"cannot connect to engine at {address} — is it running?")
    finally:
        ch.close()


def _die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)


# Argument parser
def _add_run_args(p: argparse.ArgumentParser) -> None:
    """Shared flags for 'run' and 'run-all'."""
    p.add_argument("--iterations", type=int,   default=None, metavar="N",
                   help="number of signal cycles (overrides config test.iterations)")
    p.add_argument("--wait",       type=float, default=None, metavar="SECS",
                   help="seconds to wait for fills before flatten (overrides config test.wait_secs)")
    p.add_argument("--no-flatten", action="store_true",
                   help="skip the end-of-run position flatten")
    p.add_argument("--verbose",    action="store_true",
                   help="show per-signal gRPC logs")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="client.py",
        description="quarcc trading engine — test client",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    p.add_argument("--config", default=DEFAULT_CONFIG, metavar="PATH",
                   help=f"config.yaml path (default: {DEFAULT_CONFIG})")
    p.add_argument("--server", default=DEFAULT_SERVER, metavar="HOST:PORT",
                   help=f"gRPC address (default: {DEFAULT_SERVER})")

    sub = p.add_subparsers(dest="cmd", required=True)

    # run
    pr = sub.add_parser("run", help="run test scenario for one strategy")
    pr.add_argument("strategy",
                    help="name or 1-based index (e.g. '1' or 'simple_test_strategy_1')")
    _add_run_args(pr)
    pr.set_defaults(func=cmd_run)

    # run-all
    pa = sub.add_parser("run-all", help="run all configured strategy scenarios")
    pa.add_argument("--parallel", action="store_true",
                    help="run all strategies concurrently")
    _add_run_args(pa)
    pa.set_defaults(func=cmd_run_all)

    # positions
    pp = sub.add_parser("positions", help="print current positions from the engine")
    pp.set_defaults(func=cmd_positions)

    # kill
    pk = sub.add_parser("kill", help="activate the engine kill switch")
    pk.add_argument("--reason", default="manual stop", help="reason (default: 'manual stop')")
    pk.add_argument("--by",     default="dev",         dest="initiated_by",
                    help="who triggered it (default: dev)")
    pk.set_defaults(func=cmd_kill)

    # gen-config
    pg = sub.add_parser(
        "gen-config",
        help="print config.yaml (with optional gateway override) — pipe output to the engine",
    )
    pg.add_argument("--gateway", choices=["paper trading", "alpaca"],
                    help="override gateway for every strategy")
    pg.set_defaults(func=cmd_gen_config)

    return p


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
