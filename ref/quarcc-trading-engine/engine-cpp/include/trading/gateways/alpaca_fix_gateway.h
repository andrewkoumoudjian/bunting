#pragma once

#include <trading/interfaces/i_execution_gateway.h>

#include <alpaca/alpaca.hpp>
#include <alpaca/client/tradingClient.hpp>

#include <unordered_map>

namespace quarcc {

class AlpacaGateway final : public GatewayDispatchBase {
public:
  AlpacaGateway();

  // TODO: Make config file for API key parsing
  // TODO: Fix warnings, probably by giving default value as std::nullopt to
  // alpaca-sdk structs
  Result<BrokerOrderId> submit_order(const v1::Order &order) override;
  Result<std::monostate> cancel_order(const BrokerOrderId &orderId) override;
  Result<BrokerOrderId> replace_order(const BrokerOrderId &orderId,
                                      const v1::Order &new_order) override;

  void start() override;
  void stop() override;

private:
  constexpr alpaca::OrderRequestParam
  order_to_alpaca_order(const v1::Order &order) const;
  constexpr alpaca::OrderSide order_enum_conversion(v1::Side type) const;
  constexpr v1::Side order_enum_conversion(alpaca::OrderSide type) const;
  constexpr alpaca::OrderType order_enum_conversion(v1::OrderType type) const;
  constexpr alpaca::OrderTimeInForce
  order_enum_conversion(v1::TimeInForce type) const;

private:
  // TODO: config.yaml takes in environemnt variables like api keys for env_.
  // Modify alpaca-sdk-cpp
  alpaca::Environment env_;
  alpaca::TradingClient trade_;
  alpaca::TradeUpdateStream stream_;

  // Cumulative qty and cost tracker
  // Only accessed from the stream callback thread so no mutex needed
  std::unordered_map<BrokerOrderId, double> cum_qty_;
  std::unordered_map<BrokerOrderId, double> cum_cost_;
};

} // namespace quarcc
