#pragma once

#include "order.pb.h"
#include "strategy_signal.pb.h"

#include <trading/core/position_keeper.h>
#include <trading/interfaces/i_execution_gateway.h>
#include <trading/interfaces/i_journal.h>
#include <trading/interfaces/i_market_data_feed.h>
#include <trading/interfaces/i_order_store.h>
#include <trading/interfaces/i_risk_check.h>
#include <trading/utils/event_queue.h>
#include <trading/utils/order_id_generator.h>
#include <trading/utils/order_id_types.h>
#include <trading/utils/result.h>

#include <functional>
#include <mutex>
#include <thread>
#include <variant>

namespace quarcc {

// Sentinel pushed onto the OM queue after id_mapper_->add_mapping() to trigger
// a deferred fill retry. Fills that arrived before the mapping was established
// are held in deferred_fills_ and replayed when this event is processed.
struct RetryFillsEvent {};

// All events processed by each order manager
// No lock required, they're all processed sequentially
using OMEvent = std::variant<Bar, Tick, v1::ExecutionReport, RetryFillsEvent>;

class OrderManager {
public:
  OrderManager(const OrderManager &) = delete;
  OrderManager &operator=(const OrderManager &) = delete;
  OrderManager(OrderManager &&) = delete;
  OrderManager &operator=(OrderManager &&) = delete;

  ~OrderManager();

  static std::unique_ptr<OrderManager> create_order_manager(
      std::string account_id, std::unique_ptr<PositionKeeper> pk,
      std::unique_ptr<IExecutionGateway> gw, std::unique_ptr<IJournal> lj,
      std::unique_ptr<IOrderStore> os, std::unique_ptr<RiskManager> rm);

  // May be called by different threads (gRPC, market feed, etc.)
  void enqueue(OMEvent event);

  // Processes all remaining events for a graceful destruction
  // Especially useful for debugging logs after a crash. SO KEEP IT!
  void wait_idle();

  // Sets sink funtions for handling data for bars and ticks given by gRPC to a
  // specific strategy. They're later called by the dispatcher thread whenever a
  // bar/tick event comes inside the queue
  // ALSO: always call clear_market_data_sinks() before the ServerWriter is
  // destroyed
  void set_market_data_sinks(std::function<void(const Tick &)> tick_sink,
                             std::function<void(const Bar &)> bar_sink);
  void clear_market_data_sinks();

  Result<LocalOrderId> process_signal(const v1::StrategySignal &signal);
  Result<std::monostate> process_signal(const v1::CancelSignal &signal);
  Result<LocalOrderId> process_signal(const v1::ReplaceSignal &signal);

  void cancel_all(const std::string &reason, const std::string &initiated_by);

  Result<v1::Position> get_position(const std::string &symbol) const;
  v1::PositionList get_all_positions() const;

private:
  OrderManager(std::string account_id, std::unique_ptr<PositionKeeper> pk,
               std::unique_ptr<IExecutionGateway> gw,
               std::unique_ptr<IJournal> lj, std::unique_ptr<IOrderStore> os,
               std::unique_ptr<RiskManager> rm);

  v1::Order create_order_from_signal(const v1::StrategySignal &signal);
  v1::Order create_order_from_signal(const v1::ReplaceSignal &signal);

  // Big dawg loop that cleans up the event queue by calling the right handler
  // for the queue's front event
  void run_dispatch_loop(std::stop_token st);

  // Handlers, ONLY CALLED BY THE DISPATCH THREAD...
  void handle_fill(const v1::ExecutionReport &fill);
  void handle_bar(const Bar &bar);
  void handle_tick(const Tick &tick);
  void handle_retry_fills();

private:
  std::string account_id_;

  std::unique_ptr<PositionKeeper> position_keeper_;
  std::unique_ptr<IExecutionGateway> gateway_;
  std::unique_ptr<IJournal> journal_;
  std::unique_ptr<IOrderStore> order_store_;
  std::unique_ptr<RiskManager> risk_manager_;
  std::unique_ptr<OrderIdGenerator> id_generator_;
  std::unique_ptr<OrderIdMapper> id_mapper_;

  // Container to store fills that arrived before the id_mapper_ registered
  // the order. Replayed via RetryFillsEvent.
  // TBD: Might not be needed anymore?
  std::vector<v1::ExecutionReport> deferred_fills_;

  // Functions set at the grpc_server level to give behavior when a data comes,
  // basically sends that data through gRPC to the client. Look at
  // gRPCServer::StreamMarketData for more info
  //
  // Mutex used to protect handling the data and setting the sink functions
  // since both the gRPC and the dispatcher thread can touch it... Dangerous!
  std::function<void(const Tick &)> tick_sink_;
  std::function<void(const Bar &)> bar_sink_;
  std::mutex md_sink_mu_;

  // IMPORTANT: dispatch_thread_ must remain last, its destructor joins the
  // thread before any other members are destroyed
  EventQueue<OMEvent> queue_;
  std::jthread dispatch_thread_;
};

} // namespace quarcc
