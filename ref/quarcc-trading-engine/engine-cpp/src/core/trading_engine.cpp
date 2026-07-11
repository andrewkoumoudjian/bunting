#include <trading/core/trading_engine.h>
#include <trading/gateways/paper_trading_gateway.h>
#include <trading/persistence/sqlite_journal.h>
#include <trading/persistence/sqlite_order_store.h>

namespace quarcc {

void TradingEngine::Run(const char *config_path) {
  GOOGLE_PROTOBUF_VERIFY_VERSION;

  try {
    process_config(config_path);
  } catch (const std::exception &e) {
    std::cerr << "Config error: " << e.what() << std::endl;
    return;
  }

  feed_registry_.start_all();
  server_->start();

  // Block until ActivateKillSwitch() sets running_ = false and notifies
  {
    std::unique_lock lk{run_mu_};
    run_cv_.wait(lk, [&] { return !running_.load(); });
  }

  server_->shutdown();
  feed_registry_.stop_all();
  google::protobuf::ShutdownProtobufLibrary();
}

// Submit/Cancel/Replace orders through custom gateway
Result<BrokerOrderId>
TradingEngine::SubmitSignal(const v1::StrategySignal &signal) {
  auto it = managers_.find(signal.strategy_id());
  if (it == managers_.end()) [[unlikely]]
    return std::unexpected(Error{"Unknown strategy", ErrorType::Error});
  return it->second->process_signal(signal);
}

Result<std::monostate>
TradingEngine::CancelOrder(const v1::CancelSignal &signal) {
  auto it = managers_.find(signal.strategy_id());
  if (it == managers_.end()) [[unlikely]]
    return std::unexpected(Error{"Unknown strategy", ErrorType::Error});
  return it->second->process_signal(signal);
}

Result<BrokerOrderId>
TradingEngine::ReplaceOrder(const v1::ReplaceSignal &signal) {
  auto it = managers_.find(signal.strategy_id());
  if (it == managers_.end()) [[unlikely]]
    return std::unexpected(Error{"Unknown strategy", ErrorType::Error});
  return it->second->process_signal(signal);
}

Result<v1::Position>
TradingEngine::GetPosition(const v1::GetPositionRequest &req) {
  v1::Position combined;
  combined.set_symbol(req.symbol());
  bool found = false;

  for (const auto &[strategy_id, manager] : managers_) {
    auto pos = manager->get_position(req.symbol());
    if (!pos)
      continue;

    found = true;
    const double existing_qty = combined.quantity();
    const double new_qty = existing_qty + pos->quantity();

    if (new_qty != 0.0 && existing_qty != 0.0) {
      combined.set_avg_price((existing_qty * combined.avg_price() +
                              pos->quantity() * pos->avg_price()) /
                             new_qty);
    } else if (pos->quantity() != 0.0) {
      combined.set_avg_price(pos->avg_price());
    }
    combined.set_quantity(new_qty);
  }

  if (!found)
    return std::unexpected(
        Error{"No position found for " + req.symbol(), ErrorType::Error});

  return combined;
}

Result<v1::PositionList> TradingEngine::GetAllPositions(const v1::Empty &) {
  std::unordered_map<std::string, v1::Position> combined;

  for (const auto &[strategy_id, manager] : managers_) {
    const v1::PositionList list = manager->get_all_positions();
    for (const auto &pos : list.positions()) {
      auto it = combined.find(pos.symbol());
      if (it == combined.end()) {
        combined[pos.symbol()] = pos;
      } else {
        v1::Position &existing = it->second;
        const double new_qty = existing.quantity() + pos.quantity();
        if (new_qty != 0.0) {
          existing.set_avg_price((existing.quantity() * existing.avg_price() +
                                  pos.quantity() * pos.avg_price()) /
                                 new_qty);
        }
        existing.set_quantity(new_qty);
      }
    }
  }

  v1::PositionList result;
  for (auto &[symbol, pos] : combined)
    result.add_positions()->CopyFrom(pos);

  return result;
}

// Moving the handling and sending of data from the gRPCServer::StreamMarketData
// function to the specific strategies
Result<std::monostate>
TradingEngine::SetupMarketDataStream(const std::string &strategy_id,
                                     MarketDataSinks sinks) {
  auto it = managers_.find(strategy_id);
  if (it == managers_.end())
    return std::unexpected(
        Error{"Unknown strategy: " + strategy_id, ErrorType::Error});
  it->second->set_market_data_sinks(std::move(sinks.on_tick),
                                    std::move(sinks.on_bar));
  return std::monostate{};
}

// Clear the handling and sending of data functions
void TradingEngine::ClearMarketDataStream(const std::string &strategy_id) {
  auto it = managers_.find(strategy_id);
  if (it != managers_.end())
    it->second->clear_market_data_sinks();
}

Result<std::monostate>
TradingEngine::ActivateKillSwitch(const v1::KillSwitchRequest &req) {
  for (auto &[strategy_id, manager] : managers_)
    manager->cancel_all(req.reason(), req.initiated_by());

  {
    std::lock_guard lk{run_mu_};
    running_ = false;
  }
  run_cv_.notify_one();

  return std::monostate{};
}

void TradingEngine::process_config(const std::string &path) {
  const Config config = parse_config(path);

  for (const auto &strat : config.strategies) {
    std::unique_ptr<IExecutionGateway> gateway;

    if (strat.gateway == "alpaca") {
#if TRADING_ENABLE_ALPACA_SDK
      gateway = std::make_unique<AlpacaGateway>();
#else
      throw std::runtime_error(
          "Alpaca gateway not compiled in (TRADING_ENABLE_ALPACA_SDK=OFF)");
#endif
    } else if (strat.gateway == "paper trading") {
      gateway = std::make_unique<PaperGateway>();
    } else {
      throw std::runtime_error("Invalid gateway: " + strat.gateway);
    }

    managers_.emplace(
        StrategyId{strat.id},
        OrderManager::create_order_manager(
            config.account_id, std::make_unique<PositionKeeper>(),
            std::move(gateway),
            std::make_unique<SQLiteJournal>(strat.database.journal),
            std::make_unique<SQLiteOrderStore>(strat.database.orders),
            std::make_unique<RiskManager>()));

    if (strat.market_data) {
      OrderManager *om = managers_.at(strat.id).get();
      FeedKey key{strat.market_data->feed, strat.account_id};

      for (const auto &sub : strat.market_data->subscriptions)
        feed_registry_.register_subscription(key, sub.symbol, sub.period, om);
    }
  }

  server_ = std::make_unique<gRPCServer>(config.network.grpc.host_post, *this);
}

} // namespace quarcc
