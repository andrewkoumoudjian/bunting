#pragma once

#include <trading/interfaces/i_execution_gateway.h>

#include <atomic>
#include <mutex>
#include <random>
#include <thread>

namespace quarcc {

struct PaperGatewayConfig {
  double fill_probability = 0.85;      // per poll, MARKET orders
  double partial_fill_min = 0.3;       // minimum fraction filled when partial
  double partial_fill_max = 0.9;       // maximum fraction filled when partial
  double rejection_probability = 0.03; // submit_order rejection rate
  double slippage_bps = 5.0;           // basis points adverse slippage
  double initial_price = 100.0;        // fallback if no price seen yet
  double price_volatility = 0.001; // std dev of per-poll price move (fraction)
};

// Instant submission/cancelation/replacement/fills with custom local ids
class PaperGateway final : public GatewayDispatchBase {
public:
  PaperGateway();

  Result<BrokerOrderId> submit_order(const v1::Order &order) override;
  Result<std::monostate> cancel_order(const BrokerOrderId &orderId) override;
  Result<BrokerOrderId> replace_order(const BrokerOrderId &orderId,
                                      const v1::Order &new_order) override;

  void start() override;
  void stop() override;

private:
  void simulate_fills();
  void advance_prices() noexcept;
  double get_or_init_price(const std::string &symbol) noexcept;
  void update_price(const std::string &symbol, double bid, double ask) noexcept;

  struct PriceState {
    double bid;
    double ask;
  };

  struct SimulatedOrder {
    v1::Order order;
    double submitted_at_price; // mid at submission time
    double cumulative_filled;  // across poll cycles
  };

private:
  std::unordered_map<BrokerOrderId, SimulatedOrder> pending_orders_;
  std::mutex orders_mutex_;
  OrderIdGenerator id_gen_;
  std::unordered_map<std::string, PriceState> prices_;
  // BUG TODO: rng_ is not thread safe
  std::mt19937 rng_{std::random_device{}()};
  std::normal_distribution<double> price_dist_{0.0, 1.0};
  PaperGatewayConfig config_;

  std::thread simulation_thread_;
  std::atomic<bool> running_{false};
};

} // namespace quarcc
