#!/usr/bin/env python3
"""Minimal demo: print every tick and bar from the simulated feed.

Start the engine first with a strategy that has market_data.feed = "simulated":

    ./build/engine-cpp/src/trading_engine python_client/config.yaml &
    python3 python_client/strategy_example.py
    # or target a specific strategy:
    python3 python_client/strategy_example.py simple_test_strategy_1
"""

import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from strategy import Strategy, run


class PrintStrategy(Strategy):
    def __init__(self):
        self.tick_count = 0
        self.bar_count  = 0

    def on_tick(self, tick) -> None:
        self.tick_count += 1
        print(
            f"[TICK {self.tick_count:4d}] {tick.symbol:<8}"
            f"  last={tick.last:9.4f}"
            f"  bid={tick.bid:9.4f}"
            f"  ask={tick.ask:9.4f}"
        )

    def on_bar(self, bar) -> None:
        self.bar_count += 1
        print(
            f"[ BAR {self.bar_count:4d}] {bar.symbol:<8}"
            f"  [{bar.period}]"
            f"  O={bar.open:.4f}"
            f"  H={bar.high:.4f}"
            f"  L={bar.low:.4f}"
            f"  C={bar.close:.4f}"
            f"  V={bar.volume:.0f}"
            f"  VWAP={bar.vwap:.4f}"
        )


if __name__ == "__main__":
    strategy_id = sys.argv[1] if len(sys.argv) > 1 else "simple_test_strategy_1"
    print(f"Subscribing to market data for: {strategy_id}")
    print("Ticks every 500 ms, bars every 10 ticks (~5 s). Ctrl+C to stop.\n")
    run(PrintStrategy(), strategy_id)
