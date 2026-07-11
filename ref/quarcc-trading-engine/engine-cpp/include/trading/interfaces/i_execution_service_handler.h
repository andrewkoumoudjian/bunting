#pragma once

#include "execution_service.pb.h"

#include <trading/interfaces/i_market_data_feed.h>
#include <trading/utils/order_id_generator.h>
#include <trading/utils/result.h>

#include <functional>
#include <variant>

namespace quarcc {

// Tick + bar callbacks passed to SetupMarketDataStream in a single call so
// they are always registered and cleared atomically on the OrderManager
struct MarketDataSinks {
  std::function<void(const Tick &)> on_tick;
  std::function<void(const Bar &)> on_bar;
};

struct IExecutionServiceHandler {
  virtual ~IExecutionServiceHandler() = default;

  virtual Result<BrokerOrderId> SubmitSignal(const v1::StrategySignal &req) = 0;
  virtual Result<std::monostate> CancelOrder(const v1::CancelSignal &req) = 0;
  virtual Result<BrokerOrderId> ReplaceOrder(const v1::ReplaceSignal &req) = 0;
  virtual Result<v1::Position>
  GetPosition(const v1::GetPositionRequest &req) = 0;
  virtual Result<v1::PositionList> GetAllPositions(const v1::Empty &req) = 0;
  virtual Result<std::monostate>
  ActivateKillSwitch(const v1::KillSwitchRequest &req) = 0;
  virtual Result<std::monostate>
  SetupMarketDataStream(const std::string &strategy_id,
                        MarketDataSinks sinks) = 0;
  virtual void ClearMarketDataStream(const std::string &strategy_id) = 0;
};

} // namespace quarcc
