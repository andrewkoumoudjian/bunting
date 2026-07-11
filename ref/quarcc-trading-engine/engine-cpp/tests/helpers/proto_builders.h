#pragma once

#include "common.pb.h"
#include "execution.pb.h"
#include "order.pb.h"
#include "strategy_signal.pb.h"
#include <trading/interfaces/i_order_store.h>

namespace quarcc::test {

inline v1::StrategySignal make_signal(const std::string &strategy_id = "TEST",
                                      const std::string &symbol = "AAPL",
                                      v1::Side side = v1::Side::BUY,
                                      double qty = 10.0,
                                      double confidence = 1.0) {
  v1::StrategySignal sig;
  sig.set_strategy_id(strategy_id);
  sig.set_symbol(symbol);
  sig.set_side(side);
  sig.set_target_quantity(qty);
  sig.set_confidence(confidence);
  return sig;
}

inline v1::Order make_order(const std::string &id = "ORD_001",
                            const std::string &symbol = "AAPL",
                            v1::Side side = v1::Side::BUY, double qty = 10.0) {
  v1::Order order;
  order.set_id(id);
  order.set_symbol(symbol);
  order.set_side(side);
  order.set_quantity(qty);
  order.set_type(v1::OrderType::MARKET);
  order.set_time_in_force(v1::TimeInForce::DAY);
  return order;
}

inline StoredOrder make_stored_order(
    const std::string &local_id = "ORD_001",
    const std::string &symbol = "AAPL", v1::Side side = v1::Side::BUY,
    double qty = 10.0, OrderStatus status = OrderStatus::SUBMITTED,
    std::optional<std::string> broker_id = std::nullopt) {
  StoredOrder stored;
  stored.local_id = local_id;
  stored.order = make_order(local_id, symbol, side, qty);
  stored.status = status;
  stored.broker_id = broker_id;
  stored.created_at = "2024-01-01 00:00:00.000";
  return stored;
}

inline v1::ExecutionReport make_fill(const std::string &broker_id = "BROKER_001",
                                     const std::string &symbol = "AAPL",
                                     v1::Side side = v1::Side::BUY,
                                     double filled_qty = 10.0,
                                     double avg_price = 150.0) {
  v1::ExecutionReport fill;
  fill.set_broker_order_id(broker_id);
  fill.set_symbol(symbol);
  fill.set_side(side);
  fill.set_filled_quantity(filled_qty);
  fill.set_avg_fill_price(avg_price);
  return fill;
}

} // namespace quarcc::test
