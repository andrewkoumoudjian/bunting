#pragma once

#include <trading/utils/order_id_generator.h>
#include <trading/utils/result.h>

#include "execution.pb.h"
#include "order.pb.h"

#include <condition_variable>
#include <functional>
#include <mutex>
#include <queue>
#include <thread>

namespace quarcc {

using FillHandler = std::function<void(const v1::ExecutionReport &)>;
using RejectHandler = std::function<void(const v1::ExecutionReport &)>;

class IExecutionGateway {
public:
  virtual ~IExecutionGateway() = default;

  virtual Result<BrokerOrderId> submit_order(const v1::Order &order) = 0;
  virtual Result<std::monostate> cancel_order(const BrokerOrderId &orderId) = 0;
  virtual Result<BrokerOrderId> replace_order(const BrokerOrderId &orderId,
                                              const v1::Order &new_order) = 0;

  virtual void set_fill_handler(FillHandler handler) = 0;
  virtual void set_reject_handler(RejectHandler handler) = 0;
  virtual void start() = 0;
  virtual void stop() = 0;
};

// Intermediate class between actual gateways and the interface to inforce the
// dispatcher thread + queue design
// Abstracted most common code
class GatewayDispatchBase : public IExecutionGateway {
public:
  void set_fill_handler(FillHandler handler) override {
    fill_handler_ = std::move(handler);
  }
  void set_reject_handler(RejectHandler handler) override {
    reject_handler_ = std::move(handler);
  }

protected:
  void enqueue_report(v1::ExecutionReport report) {
    {
      std::lock_guard lk{dispatch_mu_};
      reports_queue_.push(std::move(report));
    }
    dispatch_cv_.notify_one();
  }

  void start_dispatcher() {
    dispatcher_thread_ = std::jthread([this](std::stop_token st) {
      v1::ExecutionReport report;
      while (true) {
        {
          std::unique_lock lk{dispatch_mu_};
          dispatch_cv_.wait(lk, [&] {
            return !reports_queue_.empty() || st.stop_requested();
          });
          if (st.stop_requested() && reports_queue_.empty())
            break;
          report = std::move(reports_queue_.front());
          reports_queue_.pop();
        }
        if (fill_handler_) [[likely]]
          fill_handler_(report);
      }
    });
  }

  void stop_dispatcher() {
    if (dispatcher_thread_.joinable()) [[likely]] {
      dispatcher_thread_.request_stop();
      dispatch_cv_.notify_all();
      dispatcher_thread_.join();
    }
  }

  FillHandler fill_handler_;
  RejectHandler reject_handler_;

private:
  std::condition_variable dispatch_cv_;
  std::mutex dispatch_mu_;
  std::queue<v1::ExecutionReport> reports_queue_;
  std::jthread dispatcher_thread_;
};

} // namespace quarcc
