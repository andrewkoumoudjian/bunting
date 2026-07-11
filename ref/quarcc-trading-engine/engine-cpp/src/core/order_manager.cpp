#include <trading/core/order_manager.h>
#include <trading/utils/event_queue.h>

namespace quarcc {

std::unique_ptr<OrderManager> OrderManager::create_order_manager(
    std::string account_id, std::unique_ptr<PositionKeeper> pk,
    std::unique_ptr<IExecutionGateway> gw, std::unique_ptr<IJournal> lj,
    std::unique_ptr<IOrderStore> os, std::unique_ptr<RiskManager> rm) {
  return std::unique_ptr<OrderManager>(
      new OrderManager(std::move(account_id), std::move(pk), std::move(gw),
                       std::move(lj), std::move(os), std::move(rm)));
}

OrderManager::OrderManager(std::string account_id,
                           std::unique_ptr<PositionKeeper> pk,
                           std::unique_ptr<IExecutionGateway> gw,
                           std::unique_ptr<IJournal> lj,
                           std::unique_ptr<IOrderStore> os,
                           std::unique_ptr<RiskManager> rm)
    : account_id_(std::move(account_id)), position_keeper_(std::move(pk)),
      gateway_(std::move(gw)), journal_(std::move(lj)),
      order_store_(std::move(os)), risk_manager_(std::move(rm)),
      id_generator_(std::make_unique<OrderIdGenerator>()),
      id_mapper_(std::make_unique<OrderIdMapper>()) {
  gateway_->set_fill_handler(
      [this](const v1::ExecutionReport &r) { enqueue(r); });

  dispatch_thread_ =
      std::jthread([this](std::stop_token st) { run_dispatch_loop(st); });

  gateway_->start();
}

// Shutdown order:
//  1. Shutdown the gateway, so it pushes all the remaining fills to the OM's
//  queue
//  2. dispatch_thread_ joins and cleans the queue up
//  3. OM gets destroyed
OrderManager::~OrderManager() { gateway_->stop(); }

void OrderManager::enqueue(OMEvent event) { queue_.push(std::move(event)); }

void OrderManager::wait_idle() { queue_.wait_idle(); }

void OrderManager::run_dispatch_loop(std::stop_token st) {
  OMEvent event;
  while (queue_.pop(event, st)) {
    std::visit(overloaded{
                   [this](const v1::ExecutionReport &r) { handle_fill(r); },
                   [this](const Bar &b) { handle_bar(b); },
                   [this](const Tick &t) { handle_tick(t); },
                   [this](RetryFillsEvent) { handle_retry_fills(); },
               },
               event);
    queue_.mark_processed();
  }
}

// Dispatch handlers. ONLY CALLED BY THE DISPATCH THREAD!

// Steps:
//  1. Resolve broker -> local ID (defer on miss)
//  2. Fetch stored order to compare filled vs original quantity
//  3. Persist fill details
//  4. Determine and persist FILLED or PARTIALLY_FILLED status
//  5. Update in-memory position
//  6. Journal the event
//  7. Remove fully-filled orders from the mapper (terminal)
void OrderManager::handle_fill(const v1::ExecutionReport &fill) {
  const std::string &broker_id = fill.broker_order_id();

  // 1. Resolve broker -> local ID
  auto local_id_opt = id_mapper_->get_local_id(broker_id);
  if (!local_id_opt) {
    // id_mapper_ didn't register the order's id yet, so we'll handle the fill
    // later
    deferred_fills_.push_back(fill);
    return;
  }
  const std::string &local_id = *local_id_opt;

  // 2. Fetch the stored order to determine full vs partial fill
  auto stored = order_store_->get_order(local_id);
  if (!stored) {
    journal_->log(Event::ERROR_OCCURRED,
                  "Cannot find stored order for local_id: " + local_id,
                  local_id);
    return;
  }

  const double filled_qty = fill.filled_quantity();
  const double original_qty = stored->order.quantity();

  // 3. Persist fill details
  if (auto r = order_store_->update_fill_info(local_id, filled_qty,
                                              fill.avg_fill_price());
      !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
  }

  // 4. Determine and persist the new order status
  const bool fully_filled =
      (stored->filled_quantity + filled_qty >= original_qty);
  const OrderStatus new_status =
      fully_filled ? OrderStatus::FILLED : OrderStatus::PARTIALLY_FILLED;

  if (auto r = order_store_->update_order_status(local_id, new_status); !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
  }

  // 5. Update in-memory position
  position_keeper_->on_fill(fill.symbol(), filled_qty, fill.avg_fill_price(),
                            fill.side());

  // 6. Journal the event
  const std::string log_data =
      std::format("Filled: {} / {} @ avg={}", filled_qty, original_qty,
                  fill.avg_fill_price());

  journal_->log(fully_filled ? Event::ORDER_FILLED
                             : Event::ORDER_PARTIALLY_FILLED,
                log_data, local_id);

  // 7. Remove fully-filled orders from the mapper since they're completely
  // filled
  if (fully_filled)
    id_mapper_->remove_mapping(local_id);
}

void OrderManager::handle_bar(const Bar &bar) {
  std::lock_guard lk{md_sink_mu_};
  if (bar_sink_) [[likely]]
    bar_sink_(bar);
}

void OrderManager::handle_tick(const Tick &tick) {
  std::lock_guard lk{md_sink_mu_};
  if (tick_sink_) [[likely]]
    tick_sink_(tick);
}

void OrderManager::set_market_data_sinks(
    std::function<void(const Tick &)> tick_sink,
    std::function<void(const Bar &)> bar_sink) {
  std::lock_guard lk{md_sink_mu_};
  tick_sink_ = std::move(tick_sink);
  bar_sink_ = std::move(bar_sink);
}

void OrderManager::clear_market_data_sinks() {
  std::lock_guard lk{md_sink_mu_};
  tick_sink_ = nullptr;
  bar_sink_ = nullptr;
}

void OrderManager::handle_retry_fills() {
  // Drain deferred_fills_ into handle_fill(). Fills that still can't resolve
  // (unlikely but possible if multiple orders are in flight) go back into
  // deferred_fills_ via handle_fill()'s defer path
  auto to_retry = std::move(deferred_fills_);
  deferred_fills_.clear();
  for (const auto &fill : to_retry)
    handle_fill(fill);
}

// All the processing of the signals that runs concurrently to the dispatch
// thread. Each component of the OM should be thread safe

Result<LocalOrderId>
OrderManager::process_signal(const v1::StrategySignal &signal) {
  std::string local_id = id_generator_->generate();
  v1::Order order = create_order_from_signal(signal);
  order.set_id(local_id);
  journal_->log(Event::ORDER_CREATED, order.DebugString(), order.id());

  StoredOrder stored;
  stored.order = order;
  stored.local_id = local_id;
  stored.status = OrderStatus::PENDING_SUBMISSION;
  stored.created_at = LogEntry::timestamp_to_string(LogEntry::now());

  if (auto r = order_store_->store_order(stored); !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
    return std::unexpected(r.error());
  }

  // TODO: Risk check

  auto result = gateway_->submit_order(order);
  if (!result) {
    journal_->log(Event::ORDER_REJECTED, result.error().message_, local_id);
    if (auto r =
            order_store_->update_order_status(local_id, OrderStatus::REJECTED);
        !r) {
      journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
      return std::unexpected(r.error());
    }
    return result;
  }

  std::string broker_id = result.value();

  // Set SUBMITTED before add_mapping() makes this order visible to the
  // dispatch thread. Without this ordering, a fill could arrive and set
  // status=FILLED, then this update would overwrite it back to SUBMITTED
  if (auto r =
          order_store_->update_order_status(local_id, OrderStatus::SUBMITTED);
      !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
    return std::unexpected(r.error());
  }

  id_mapper_->add_mapping(local_id, broker_id);

  // Signal the dispatch thread to retry any fills that arrived before the
  // mapping was established (the deferred fill race window)
  enqueue(RetryFillsEvent{});

  if (auto r = order_store_->update_broker_id(local_id, broker_id); !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
    return std::unexpected(r.error());
  }

  journal_->log(Event::ORDER_SUBMITTED,
                "Local: " + local_id + ", Broker: " + broker_id, local_id);

  return local_id;
}

Result<std::monostate>
OrderManager::process_signal(const v1::CancelSignal &signal) {
  std::string local_id = signal.order_id();

  auto broker_id = id_mapper_->get_broker_id(local_id);
  if (!broker_id) [[unlikely]] {
    return std::unexpected(Error{"Cannot find broker ID for order: " + local_id,
                                 ErrorType::Error});
  }

  auto result = gateway_->cancel_order(*broker_id);

  if (result) {
    journal_->log(Event::ORDER_CANCELLED, "Cancelled", local_id);
    id_mapper_->remove_mapping(local_id);
    if (auto r =
            order_store_->update_order_status(local_id, OrderStatus::CANCELLED);
        !r) {
      journal_->log(Event::ERROR_OCCURRED, r.error().message_, local_id);
      return std::unexpected(r.error());
    }
  }

  return result;
}

Result<LocalOrderId>
OrderManager::process_signal(const v1::ReplaceSignal &signal) {
  std::string old_local_id = signal.order_id();

  auto old_broker_id = id_mapper_->get_broker_id(old_local_id);
  if (!old_broker_id) [[unlikely]] {
    return std::unexpected(Error{
        "Cannot find broker ID for order: " + old_local_id, ErrorType::Error});
  }

  std::string new_local_id = id_generator_->generate();
  v1::Order new_order = create_order_from_signal(signal);
  new_order.set_id(new_local_id);

  journal_->log(Event::ORDER_REPLACED,
                "Replacing " + old_local_id + " with " + new_local_id,
                new_local_id);

  auto result = gateway_->replace_order(*old_broker_id, new_order);
  if (!result) {
    journal_->log(Event::ORDER_REJECTED, result.error().message_, new_local_id);
    return std::unexpected(result.error());
  }

  std::string new_broker_id = result.value();
  id_mapper_->remove_mapping(old_local_id);

  if (auto r = order_store_->update_order_status(old_local_id,
                                                 OrderStatus::REPLACED);
      !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, old_local_id);
    return std::unexpected(r.error());
  }

  id_mapper_->add_mapping(new_local_id, new_broker_id);
  enqueue(RetryFillsEvent{});

  StoredOrder stored;
  stored.order = new_order;
  stored.local_id = new_local_id;
  stored.broker_id = new_broker_id;
  stored.status = OrderStatus::SUBMITTED;
  stored.created_at = LogEntry::timestamp_to_string(LogEntry::now());

  if (auto r = order_store_->store_order(stored); !r) {
    journal_->log(Event::ERROR_OCCURRED, r.error().message_, new_local_id);
    return std::unexpected(r.error());
  }

  journal_->log(Event::ORDER_SUBMITTED,
                std::format("Old: {} -> New: {} (Broker: {})", old_local_id,
                            new_local_id, new_broker_id),
                new_local_id);

  return new_local_id;
}

void OrderManager::cancel_all(const std::string &reason,
                              const std::string &initiated_by) {
  const std::string ks_data =
      "Kill switch activated by: " + initiated_by + ". Reason: " + reason;
  journal_->log(Event::KILL_SWITCH_ACTIVATED, ks_data);

  const auto open_orders = order_store_->get_open_orders();
  for (const auto &stored : open_orders) {
    if (!stored.broker_id)
      continue;

    if (auto r = gateway_->cancel_order(*stored.broker_id); r) {
      if (auto r = order_store_->update_order_status(stored.local_id,
                                                     OrderStatus::CANCELLED);
          !r) [[unlikely]] {
        journal_->log(Event::ERROR_OCCURRED, r.error().message_,
                      stored.local_id);
        continue;
      }
      id_mapper_->remove_mapping(stored.local_id);
      journal_->log(Event::ORDER_CANCELLED, "Cancelled by kill switch",
                    stored.local_id);
    } else [[unlikely]] {
      journal_->log(Event::ERROR_OCCURRED,
                    "Failed to cancel during kill switch: " +
                        r.error().message_,
                    stored.local_id);
    }
  }
}

Result<v1::Position>
OrderManager::get_position(const std::string &symbol) const {
  return position_keeper_->get_position(symbol);
}

v1::PositionList OrderManager::get_all_positions() const {
  return position_keeper_->get_all_positions();
}

v1::Order
OrderManager::create_order_from_signal(const v1::StrategySignal &signal) {
  v1::Order order;
  order.set_symbol(signal.symbol());
  order.set_side(signal.side());
  order.set_quantity(signal.target_quantity());
  order.set_type(v1::OrderType::MARKET);
  order.set_account_id("quarcc.Rifat");
  order.set_created_at(get_current_time());
  order.set_time_in_force(v1::TimeInForce::DAY);
  order.set_strategy_id(signal.strategy_id());
  return order;
}

v1::Order
OrderManager::create_order_from_signal(const v1::ReplaceSignal &signal) {
  v1::Order order;
  order.set_symbol(signal.symbol());
  order.set_side(signal.side());
  order.set_quantity(signal.target_quantity());
  order.set_type(v1::OrderType::MARKET);
  order.set_account_id("quarcc.Rifat");
  order.set_created_at(get_current_time());
  order.set_time_in_force(v1::TimeInForce::DAY);
  order.set_strategy_id(signal.strategy_id());
  return order;
}

} // namespace quarcc
