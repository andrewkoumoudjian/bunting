#pragma once

#include <gmock/gmock.h>
#include <trading/interfaces/i_execution_gateway.h>

namespace quarcc {

class MockExecutionGateway : public IExecutionGateway {
public:
  MOCK_METHOD(Result<BrokerOrderId>, submit_order, (const v1::Order &order),
              (override));
  MOCK_METHOD(Result<std::monostate>, cancel_order,
              (const BrokerOrderId &orderId), (override));
  MOCK_METHOD(Result<BrokerOrderId>, replace_order,
              (const BrokerOrderId &orderId, const v1::Order &new_order),
              (override));

  // Lifecycle and handler registration,implemented directly (not mocked)
  // so trigger_fill can call the stored handler synchronously
  void set_fill_handler(FillHandler h) override {
    fill_handler_ = std::move(h);
  }
  void set_reject_handler(RejectHandler) override {}
  void start() override {}
  void stop() override {}

  // Test helper: call the registered fill handler synchronously in the
  // calling thread, making fill related tests deterministic without sleeps
  void trigger_fill(const v1::ExecutionReport &report) {
    if (fill_handler_)
      fill_handler_(report);
  }

private:
  FillHandler fill_handler_;
};

} // namespace quarcc
