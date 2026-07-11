#include <trading/gateways/paper_trading_gateway.h>

#include <chrono>
#include <stop_token>

namespace quarcc {

PaperGateway::PaperGateway() : id_gen_("BROKER") {}

Result<BrokerOrderId> PaperGateway::submit_order(const v1::Order &order) {
  std::uniform_real_distribution<double> dist{0.0, 1.0};

  {
    std::lock_guard lk{orders_mutex_};

    // rng_ is not thread safe, so protecting it with a mutex is important
    if (dist(rng_) < config_.rejection_probability) [[unlikely]] {
      return std::unexpected(Error{"Simulated order submittion rejection",
                                   ErrorType::FailedOrder});
    }

    const BrokerOrderId broker_id = id_gen_.generate();
    pending_orders_[broker_id] = SimulatedOrder{
        .order = order,
        .submitted_at_price = get_or_init_price(order.symbol()),
        .cumulative_filled = 0.0,
    };
    return broker_id;
  }
}

Result<std::monostate>
PaperGateway::cancel_order(const BrokerOrderId &orderId) {
  {
    std::lock_guard lk{orders_mutex_};
    pending_orders_.erase(orderId);
  }

  return std::monostate{};
}

Result<BrokerOrderId> PaperGateway::replace_order(const BrokerOrderId &orderId,
                                                  const v1::Order &new_order) {
  std::uniform_real_distribution<double> dist{0.0, 1.0};

  {
    std::lock_guard lk{orders_mutex_};

    if (dist(rng_) < config_.rejection_probability) {
      return std::unexpected(Error{"Simulated order replacement rejection",
                                   ErrorType::FailedOrder});
    }

    const BrokerOrderId broker_id = id_gen_.generate();
    pending_orders_.erase(orderId);
    pending_orders_[broker_id] = SimulatedOrder{
        .order = new_order,
        .submitted_at_price = get_or_init_price(new_order.symbol()),
        .cumulative_filled = 0.0,
    };
    return broker_id;
  }
}

void PaperGateway::start() {
  running_ = true;

  start_dispatcher();

  simulation_thread_ = std::thread([this] {
    while (running_) {
      simulate_fills();
      std::this_thread::sleep_for(std::chrono::milliseconds{100});
    }
  });
}

void PaperGateway::stop() {
  running_ = false;
  if (simulation_thread_.joinable()) [[likely]]
    simulation_thread_.join();
  stop_dispatcher();
}

void PaperGateway::simulate_fills() {
  std::vector<BrokerOrderId> fully_filled;
  std::uniform_real_distribution<double> dist{0.0, 1.0};
  std::vector<v1::ExecutionReport> generated_fills_;

  {
    std::lock_guard lk{orders_mutex_};
    generated_fills_.reserve(pending_orders_.size());
    advance_prices();

    for (auto &[broker_id, sim] : pending_orders_) {
      const v1::Order &order = sim.order;
      const std::string &symbol = order.symbol();

      // Shouldnt happen
      if (!prices_.contains(symbol)) {
        get_or_init_price(symbol);
      }
      const PriceState &ps = prices_[symbol];
      const double mid = (ps.bid + ps.ask) / 2.0;

      // LIMIT order fill conditions
      // limit buy only fills if ask is same or below the price
      // limit ask only fills if bid is same or above the price
      if (order.type() == v1::OrderType::LIMIT) {
        bool fillable =
            (order.side() == v1::Side::BUY && ps.ask <= order.price()) ||
            (order.side() == v1::Side::SELL && ps.bid >= order.price());
        if (!fillable)
          continue;
      }

      // MARKET has a configurable fill probability
      // LIMIT has a predetermined high 95% fill probability
      double fill_prob = (order.type() == v1::OrderType::LIMIT)
                             ? 0.95
                             : config_.fill_probability;
      if (dist(rng_) > fill_prob)
        continue;

      const double remaining = order.quantity() - sim.cumulative_filled;
      double fill_qty;

      // 70% of filling the remaining quantity fully
      // 30% of a partial fill of the remaining
      if (dist(rng_) < 0.7) {
        fill_qty = remaining;
      } else {
        std::uniform_real_distribution<double> partial_dist{
            config_.partial_fill_min, config_.partial_fill_max};
        fill_qty = std::floor(remaining * partial_dist(rng_));
        if (fill_qty < 1.0)
          fill_qty = 1.0;
      }
      fill_qty = std::min(fill_qty, remaining);

      // determine fill price
      // LIMIT fills at the limit price
      // MARKET fills with a slippage involved
      double fill_price;
      if (order.type() == v1::OrderType::LIMIT) {
        fill_price = order.price();
      } else {
        double slippage = mid * (config_.slippage_bps / 10000.0);
        fill_price = (order.side() == v1::Side::BUY) ? ps.ask + slippage
                                                     : ps.bid - slippage;
      }

      sim.cumulative_filled += fill_qty;
      const bool fully = (sim.cumulative_filled >= order.quantity());

      v1::ExecutionReport report;
      report.set_broker_order_id(broker_id);
      report.set_symbol(symbol);
      report.set_side(order.side());
      report.set_filled_quantity(fill_qty);
      report.set_avg_fill_price(fill_price);
      report.set_fill_time(get_current_time());

      generated_fills_.push_back(std::move(report));

      if (fully)
        fully_filled.push_back(broker_id);
    }

    for (const auto &id : fully_filled)
      pending_orders_.erase(id);
  }

  for (auto &report : generated_fills_)
    enqueue_report(std::move(report));
}

void PaperGateway::advance_prices() noexcept {
  constexpr double spread_bps = 5.0;
  for (auto &[symbol, ps] : prices_) {
    double mid = (ps.bid + ps.ask) / 2.0;
    // new move = current price * relative volatility * random shock (from news,
    // random fluctuations, etc.)
    double move = mid * config_.price_volatility * price_dist_(rng_);
    mid = std::max(mid + move, 0.01);
    // spread bps / 10000 (to get the % of the bps) / 2 (to get the half spread
    // of the %)
    double half_spread = mid * (spread_bps / 20000.0);
    ps.bid = mid - half_spread;
    ps.ask = mid + half_spread;
  }
}

double PaperGateway::get_or_init_price(const std::string &symbol) noexcept {
  if (!prices_.contains(symbol)) {
    double mid = config_.initial_price;
    double half_spread = mid * 0.00025;
    prices_[symbol] = {mid - half_spread, mid + half_spread};
  }

  return (prices_[symbol].bid + prices_[symbol].ask) / 2.0;
}

// TODO: MarketDataService that would call this
void PaperGateway::update_price(const std::string &symbol, double bid,
                                double ask) noexcept {
  std::lock_guard lk{orders_mutex_};
  prices_[symbol] = {bid, ask};
}

} // namespace quarcc
