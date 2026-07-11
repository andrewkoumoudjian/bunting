#pragma once

#include <trading/core/feed_registry.h>
#include <trading/core/order_manager.h>
#if TRADING_ENABLE_ALPACA_SDK
#include <trading/gateways/alpaca_fix_gateway.h>
#endif
#include <trading/grpc/grpc_server.h>
#include <trading/interfaces/i_execution_service_handler.h>
#include <trading/utils/config.h>
#include <trading/utils/order_id_generator.h>

#include <condition_variable>
#include <memory>
#include <mutex>

namespace quarcc {

class TradingEngine final : public IExecutionServiceHandler {
  using StrategyId = std::string;

public:
  void Run(const char *config_path);

private:
  void process_config(const std::string &path);

private:
  // Service handler
  Result<BrokerOrderId> SubmitSignal(const v1::StrategySignal &req) override;
  Result<std::monostate> CancelOrder(const v1::CancelSignal &req) override;
  Result<BrokerOrderId> ReplaceOrder(const v1::ReplaceSignal &req) override;
  Result<v1::Position> GetPosition(const v1::GetPositionRequest &req) override;
  Result<v1::PositionList> GetAllPositions(const v1::Empty &req) override;
  Result<std::monostate>
  ActivateKillSwitch(const v1::KillSwitchRequest &req) override;
  Result<std::monostate> SetupMarketDataStream(const std::string &strategy_id,
                                               MarketDataSinks sinks) override;
  void ClearMarketDataStream(const std::string &strategy_id) override;

private:
  std::atomic<bool> running_{true};

  std::mutex run_mu_;
  std::condition_variable run_cv_;

  FeedRegistry feed_registry_;

  std::unique_ptr<gRPCServer> server_;
  std::unordered_map<StrategyId, std::unique_ptr<OrderManager>> managers_;
};

} // namespace quarcc
