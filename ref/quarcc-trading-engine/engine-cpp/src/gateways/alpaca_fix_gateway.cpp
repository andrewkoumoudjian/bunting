#include <trading/gateways/alpaca_fix_gateway.h>

namespace quarcc {

AlpacaGateway::AlpacaGateway() : trade_(env_), stream_(env_) {}

Result<BrokerOrderId> AlpacaGateway::submit_order(const v1::Order &order) {
  const auto o = order_to_alpaca_order(order);
  auto resp = trade_.SubmitOrder(o);
  if (!resp) {
    return std::unexpected(Error{resp.error().message, ErrorType::Error});
  }

  return resp->id;
}

Result<std::monostate>
AlpacaGateway::cancel_order(const BrokerOrderId &orderId) {
  auto resp = trade_.DeleteOrderByID(orderId);
  if (!resp) {
    return std::unexpected(Error{resp.error().message, ErrorType::Error});
  }

  return std::monostate{};
}

Result<BrokerOrderId> AlpacaGateway::replace_order(const BrokerOrderId &orderId,
                                                   const v1::Order &new_order) {
  const alpaca::ReplaceOrderParam replace{
      .qty = std::make_optional(new_order.quantity()),
  };

  auto resp = trade_.ReplaceOrderByID(orderId, replace);
  if (!resp) {
    return std::unexpected(Error{resp.error().message, ErrorType::Error});
  }

  return resp->id;
}

void AlpacaGateway::start() {
  start_dispatcher();

  alpaca::TradeUpdateCallbacks cbs;
  cbs.onUpdate = [this](const alpaca::TradeUpdate &u) {
    using namespace alpaca;
    const auto &broker_id = u.order.id;

    if (u.event == TradeUpdateEvent::fill ||
        u.event == TradeUpdateEvent::partial_fill) {
      // Alpaca reports cumulative filledQty/filledAvgPrice across partial fills
      // Convert to incremental values the engine expects
      const double new_cum_qty = std::stod(u.order.filledQty);
      const double new_cum_avg = std::stod(u.order.filledAvgPrice.value());
      const double new_cum_cost = new_cum_qty * new_cum_avg;

      const auto qty_it = cum_qty_.find(broker_id);
      const auto cost_it = cum_cost_.find(broker_id);
      const double prev_qty = (qty_it != cum_qty_.end()) ? qty_it->second : 0.0;
      const double prev_cost =
          (cost_it != cum_cost_.end()) ? cost_it->second : 0.0;

      const double incr_qty = new_cum_qty - prev_qty;
      const double incr_price = (incr_qty > 0.0)
                                    ? (new_cum_cost - prev_cost) / incr_qty
                                    : new_cum_avg;

      if (u.event == TradeUpdateEvent::fill) {
        cum_qty_.erase(broker_id);
        cum_cost_.erase(broker_id);
      } else {
        cum_qty_[broker_id] = new_cum_qty;
        cum_cost_[broker_id] = new_cum_cost;
      }

      v1::ExecutionReport er;
      er.set_broker_order_id(broker_id);
      er.set_symbol(u.order.symbol.value());
      er.set_side(order_enum_conversion(u.order.side));
      er.set_filled_quantity(incr_qty);
      er.set_avg_fill_price(incr_price);

      enqueue_report(std::move(er));
    } else if (u.event == TradeUpdateEvent::canceled ||
               u.event == TradeUpdateEvent::expired ||
               u.event == TradeUpdateEvent::replaced) {
      // Terminal non fill events: discard any accumulated state
      cum_qty_.erase(broker_id);
      cum_cost_.erase(broker_id);
    } else if (u.event == TradeUpdateEvent::rejected) {
      // TBD TODO: Implement reject handling
    }
  };
  stream_.Connect(std::move(cbs));
}

void AlpacaGateway::stop() {
  // TODO: unsubscribe from Alpaca order stream
  stream_.Disconnect();
  stop_dispatcher();
}

constexpr alpaca::OrderRequestParam
AlpacaGateway::order_to_alpaca_order(const v1::Order &order) const {
  return alpaca::OrderRequestParam{
      .symbol = order.symbol(),
      .amt = alpaca::ShareAmount{alpaca::Quantity{order.quantity()}},
      .side = order_enum_conversion(order.side()),
      .type = order_enum_conversion(order.type()),
      .timeInForce = order_enum_conversion(order.time_in_force()),
  };
}

constexpr alpaca::OrderSide
AlpacaGateway::order_enum_conversion(v1::Side type) const {
  switch (type) {
  case v1::Side::BUY:
    return alpaca::OrderSide::buy;
  case v1::Side::SELL:
    return alpaca::OrderSide::sell;
  default:
    throw std::invalid_argument("Unknown v1::Side");
  }
}

constexpr v1::Side
AlpacaGateway::order_enum_conversion(alpaca::OrderSide type) const {
  switch (type) {
  case alpaca::OrderSide::buy:
    return v1::Side::BUY;
  case alpaca::OrderSide::sell:
    return v1::Side::SELL;
  default:
    throw std::invalid_argument("Unknown v1::Side");
  }
}

constexpr alpaca::OrderType
AlpacaGateway::order_enum_conversion(v1::OrderType type) const {
  switch (type) {
  case v1::OrderType::MARKET:
    return alpaca::OrderType::market;
  case v1::OrderType::LIMIT:
    return alpaca::OrderType::limit;
  case v1::OrderType::STOP_LIMIT:
    return alpaca::OrderType::stop_limit;
  case v1::OrderType::STOP:
    return alpaca::OrderType::stop;
  default:
    throw std::invalid_argument("Unknown v1::OrderType");
  }
}

constexpr alpaca::OrderTimeInForce
AlpacaGateway::order_enum_conversion(v1::TimeInForce type) const {
  switch (type) {
  case v1::TimeInForce::DAY:
    return alpaca::OrderTimeInForce::day;
  case v1::TimeInForce::FOK:
    return alpaca::OrderTimeInForce::fok;
  case v1::TimeInForce::GTC:
    return alpaca::OrderTimeInForce::gtc;
  case v1::TimeInForce::IOC:
    return alpaca::OrderTimeInForce::ioc;
  default:
    throw std::invalid_argument("Unknown v1::TimeInForce");
  }
}

} // namespace quarcc
