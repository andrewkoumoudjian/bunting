#pragma once

#include <trading/interfaces/i_market_data_feed.h>

#include <functional>
#include <thread>
#include <vector>

namespace quarcc {

// A thread computing simulated prices starting at 100.0 that move with a random
// walk streams the data
class SimulatedFeed final : public IMarketDataFeed {
public:
  void start() override;
  void stop() override;

  void subscribe(const Symbol &symbol, const BarPeriod &period) override;
  void unsubscribe(const Symbol &symbol, const BarPeriod &period) override;

  void set_bar_handler(std::function<void(const Bar &)> handler) override;
  void set_tick_handler(std::function<void(const Tick &)> handler) override;

private:
  void emit_loop(std::stop_token st);

  std::function<void(const Tick &)> tick_handler_;
  std::function<void(const Bar &)> bar_handler_;
  std::vector<Symbol> symbols_;
  std::jthread thread_;
};

} // namespace quarcc
