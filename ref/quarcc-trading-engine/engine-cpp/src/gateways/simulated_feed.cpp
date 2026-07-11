#include <trading/gateways/simulated_feed.h>

#include <algorithm>
#include <chrono>
#include <random>
#include <thread>
#include <unordered_map>

namespace quarcc {

void SimulatedFeed::start() {
  thread_ = std::jthread([this](std::stop_token st) { emit_loop(st); });
}

void SimulatedFeed::stop() {
  if (thread_.joinable()) {
    thread_.request_stop();
    thread_.join();
  }
}

void SimulatedFeed::subscribe(const Symbol &symbol, const BarPeriod &) {
  symbols_.push_back(symbol);
}

void SimulatedFeed::unsubscribe(const Symbol &symbol, const BarPeriod &) {
  symbols_.erase(std::remove(symbols_.begin(), symbols_.end(), symbol),
                 symbols_.end());
}

void SimulatedFeed::set_bar_handler(std::function<void(const Bar &)> handler) {
  bar_handler_ = std::move(handler);
}

void SimulatedFeed::set_tick_handler(
    std::function<void(const Tick &)> handler) {
  tick_handler_ = std::move(handler);
}

// TODO: bars!!!
void SimulatedFeed::emit_loop(std::stop_token st) {
  if (!tick_handler_ || symbols_.empty()) [[unlikely]]
    return;

  std::mt19937_64 rng{std::random_device{}()};
  std::normal_distribution<double> noise{0.0, 0.10}; // ~ +/-10bp per tick

  std::unordered_map<Symbol, double> prices;
  for (const auto &sym : symbols_) {
    prices[sym] = 100.0;
  }

  while (!st.stop_requested()) {
    const auto now_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
                            std::chrono::system_clock::now().time_since_epoch())
                            .count();

    for (const auto &sym : symbols_) {
      double &price = prices[sym];
      price += noise(rng);
      if (price < 1.0)
        price = 1.0;

      Tick tick;
      tick.symbol = sym;
      tick.last = price;
      tick.bid = price - 0.01;
      tick.ask = price + 0.01;
      tick.bid_size = tick.ask_size = tick.last_size = 100.0;
      tick.ts_ns = now_ns;
      tick.ts_recv_ns = now_ns;
      tick_handler_(tick);
    }

    std::this_thread::sleep_for(std::chrono::milliseconds(350));
  }
}

} // namespace quarcc
