#pragma once

#include <cstdint>
#include <functional>
#include <string>

namespace quarcc {

using Symbol = std::string;
using BarPeriod = std::string;

struct Bar {
  Symbol symbol;
  BarPeriod period;
  double open, high, low, close;
  double volume, vwap;
  int64_t ts_ns;      // bar open timestamp (nanoseconds since epoch)
  int64_t ts_recv_ns; // when the engine received this bar (latency measurement)
};

struct Tick {
  Symbol symbol;
  double bid, ask, last;
  double bid_size, ask_size, last_size;
  int64_t ts_ns;
  int64_t ts_recv_ns;
};

class IMarketDataFeed {
public:
  virtual ~IMarketDataFeed() = default;

  virtual void start() = 0;
  virtual void stop() = 0;

  // Subscribe to bar data for a symbol at the given period (e.g. "1m", "1d").
  virtual void subscribe(const Symbol &symbol, const BarPeriod &period) = 0;
  virtual void unsubscribe(const Symbol &symbol, const BarPeriod &period) = 0;

  // Handlers are set by FeedRegistry to fan out into OM queues.
  virtual void set_bar_handler(std::function<void(const Bar &)> handler) = 0;
  virtual void set_tick_handler(std::function<void(const Tick &)> handler) = 0;
};

} // namespace quarcc
