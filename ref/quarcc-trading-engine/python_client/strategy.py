"""Strategy base class and market-data streaming helper.

Usage
-----
1. Subclass Strategy and override on_tick() and/or on_bar().
2. Call run(strategy, strategy_id) to connect and start receiving events.

Example::

    class MyStrategy(Strategy):
        def on_tick(self, tick):
            print(f"{tick.symbol}  last={tick.last:.4f}")

        def on_bar(self, bar):
            print(f"{bar.symbol} [{bar.period}]  O={bar.open:.4f}  C={bar.close:.4f}")

    run(MyStrategy(), "simple_test_strategy_1")
"""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import grpc_interface


class Strategy:
    """Base class for Python strategies.

    Override on_tick and/or on_bar to react to market data.
    Both methods receive the inner message (TickEvent or BarEvent protobuf),
    not the outer MarketDataEvent envelope.
    """

    def on_tick(self, tick) -> None:
        """Called for every TickEvent received from the engine."""
        pass

    def on_bar(self, bar) -> None:
        """Called for every BarEvent received from the engine."""
        pass


def run(
    strategy: Strategy,
    strategy_id: str,
    server_address: str = "localhost:50051",
) -> None:
    """Connect to the engine and dispatch market data to *strategy*.

    Blocks until the stream ends (engine shutdown or keyboard interrupt).
    """
    client = grpc_interface.ExecutionClient(server_address)
    try:
        client.stream_market_data(
            strategy_id,
            on_tick=strategy.on_tick,
            on_bar=strategy.on_bar,
        )
    except KeyboardInterrupt:
        pass
    finally:
        client.close()
