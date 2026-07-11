#include <trading/grpc/grpc_server.h>
#include <trading/utils/order_id_generator.h>

#include <atomic>
#include <chrono>
#include <iostream>
#include <memory>
#include <thread>
#include <utility>

namespace quarcc {

gRPCServer::gRPCServer(std::string server_address,
                       IExecutionServiceHandler &handler)
    : server_address_(std::move(server_address)), handler_(&handler) {}

void gRPCServer::start() {
  service_ = std::make_unique<ExecutionServiceImpl>(this);

  grpc::ServerBuilder builder;
  builder.AddListeningPort(server_address_, grpc::InsecureServerCredentials());
  builder.RegisterService(service_.get());

  server_ = builder.BuildAndStart();
  std::cout << "gRPC server listening on " << server_address_ << std::endl;
}

void gRPCServer::wait() {
  if (server_)
    server_->Wait();
}

void gRPCServer::shutdown() {
  if (server_)
    server_->Shutdown();
}

gRPCServer::ExecutionServiceImpl::ExecutionServiceImpl(gRPCServer *owner)
    : owner_(owner) {}

grpc::Status gRPCServer::ExecutionServiceImpl::SubmitSignal(
    grpc::ServerContext *context, const v1::StrategySignal *request,
    v1::SubmitSignalResponse *response) {

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    response->set_accepted(false);
    response->set_rejection_reason("Server handler not initialized");
    return grpc::Status(grpc::ABORTED, "Server handler not initialized");
  }

  std::cout << "Received signal from " << context->peer() << " - "
            << request->strategy_id() << " " << request->side() << " "
            << request->symbol() << std::endl;

  auto r = owner_->handler_->SubmitSignal(*request);

  if (!r) {
    response->set_accepted(false);
    response->set_rejection_reason(r.error().message_);
    return grpc::Status(grpc::INVALID_ARGUMENT, r.error().message_);
  }

  response->set_accepted(true);
  response->set_order_id(r.value());

  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::CancelOrder(
    grpc::ServerContext *context, const v1::CancelSignal *request,
    v1::CancelOrderResponse *response) {

  response->set_received_at(get_current_time());

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    response->set_accepted(false);
    response->set_rejection_reason("Server handler not initialized");
    return grpc::Status(grpc::ABORTED, "Server handler not initialized");
  }

  std::cout << "Cancel signal received from " << context->peer() << " - "
            << request->strategy_id() << " " << request->order_id()
            << std::endl;

  auto r = owner_->handler_->CancelOrder(*request);

  if (!r) {
    response->set_accepted(false);
    response->set_rejection_reason(r.error().message_);
    return grpc::Status(grpc::INVALID_ARGUMENT, r.error().message_);
  }

  response->set_accepted(true);

  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::ReplaceOrder(
    grpc::ServerContext *context, const v1::ReplaceSignal *request,
    v1::ReplaceOrderResponse *response) {

  response->set_received_at(get_current_time());

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    response->set_accepted(false);
    response->set_rejection_reason("Server handler not initialized");
    return grpc::Status(grpc::ABORTED, "Server handler not initialized");
  }

  std::cout << "Replace signal received from " << context->peer() << " - "
            << request->strategy_id() << " " << request->side() << " "
            << request->symbol() << request->order_id() << std::endl;

  auto r = owner_->handler_->ReplaceOrder(*request);

  if (!r) {
    response->set_accepted(false);
    response->set_rejection_reason(r.error().message_);
    return grpc::Status(grpc::INVALID_ARGUMENT, r.error().message_);
  }

  response->set_accepted(true);
  response->set_order_id(r.value());

  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::StreamSignals(
    grpc::ServerContext *context,
    grpc::ServerReaderWriter<v1::SubmitSignalResponse, v1::StrategySignal>
        *stream) {

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    return grpc::Status(grpc::FAILED_PRECONDITION,
                        "Server handler not initialized");
  }

  std::cout << "Client " << context->peer() << " opened signal stream";

  v1::StrategySignal signal;
  while (stream->Read(&signal)) {
    auto r = owner_->handler_->SubmitSignal(signal);

    v1::SubmitSignalResponse response;
    if (!r) {
      response.set_accepted(false);
      response.set_rejection_reason(r.error().message_);
    } else {
      response.set_accepted(true);
      response.set_order_id(r.value());
    }

    stream->Write(response);
  }

  std::cout << "Client " << context->peer() << " closed signal stream\n";
  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::GetPosition(
    grpc::ServerContext *context, const v1::GetPositionRequest *request,
    v1::Position *response) {

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    return grpc::Status(grpc::StatusCode::FAILED_PRECONDITION,
                        "Server handler not initialized");
  }

  std::cout << "Received position request from " << context->peer()
            << std::endl;

  auto r = owner_->handler_->GetPosition(*request);
  if (!r)
    return grpc::Status(grpc::StatusCode::INTERNAL, r.error().message_);

  *response = std::move(r.value());
  return grpc::Status::OK;
}

grpc::Status
gRPCServer::ExecutionServiceImpl::GetAllPositions(grpc::ServerContext *context,
                                                  const v1::Empty *request,
                                                  v1::PositionList *response) {

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    return grpc::Status(grpc::StatusCode::FAILED_PRECONDITION,
                        "Server handler not initialized");
  }

  std::cout << "Received position request for all from " << context->peer()
            << std::endl;

  auto r = owner_->handler_->GetAllPositions(*request);
  if (!r) [[unlikely]]
    return grpc::Status(grpc::StatusCode::INTERNAL, r.error().message_);

  *response = std::move(r.value());
  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::ActivateKillSwitch(
    grpc::ServerContext *context, const v1::KillSwitchRequest *request,
    v1::Empty *response) {

  (void)response;

  if (!owner_ || !owner_->handler_) [[unlikely]] {
    return grpc::Status(grpc::StatusCode::FAILED_PRECONDITION,
                        "Server handler not initialized");
  }

  std::cout << "Received kill switch request from " << context->peer()
            << std::endl;

  auto r = owner_->handler_->ActivateKillSwitch(*request);
  if (!r) [[unlikely]]
    return grpc::Status(grpc::StatusCode::INTERNAL, r.error().message_);

  return grpc::Status::OK;
}

grpc::Status gRPCServer::ExecutionServiceImpl::StreamMarketData(
    grpc::ServerContext *context, const v1::SubscribeMarketDataRequest *request,
    grpc::ServerWriter<v1::MarketDataEvent> *writer) {

  if (!owner_ || !owner_->handler_) {
    return grpc::Status(grpc::StatusCode::FAILED_PRECONDITION,
                        "Server handler not initialized");
  }

  const std::string &strategy_id = request->strategy_id();

  // Active flag, set to false whenver .Write() fails, which stops getting data
  auto active = std::make_shared<std::atomic<bool>>(true);

  // Functions that the OM call when their dispatcher thread receives a bar/tick
  // event
  MarketDataSinks sinks;

  sinks.on_tick = [writer, active](const Tick &t) {
    if (!active->load(std::memory_order_relaxed))
      return;
    v1::MarketDataEvent evt;
    v1::TickEvent *tick = evt.mutable_tick();
    tick->set_symbol(t.symbol);
    tick->set_bid(t.bid);
    tick->set_ask(t.ask);
    tick->set_last(t.last);
    tick->set_bid_size(t.bid_size);
    tick->set_ask_size(t.ask_size);
    tick->set_last_size(t.last_size);
    tick->set_timestamp_ns(t.ts_ns);
    if (!writer->Write(evt))
      active->store(false, std::memory_order_relaxed);
  };

  sinks.on_bar = [writer, active](const Bar &b) {
    if (!active->load(std::memory_order_relaxed))
      return;
    v1::MarketDataEvent evt;
    v1::BarEvent *bar = evt.mutable_bar();
    bar->set_symbol(b.symbol);
    bar->set_period(b.period);
    bar->set_open(b.open);
    bar->set_high(b.high);
    bar->set_low(b.low);
    bar->set_close(b.close);
    bar->set_volume(b.volume);
    bar->set_vwap(b.vwap);
    bar->set_timestamp_ns(b.ts_ns);
    if (!writer->Write(evt))
      active->store(false, std::memory_order_relaxed);
  };

  auto r =
      owner_->handler_->SetupMarketDataStream(strategy_id, std::move(sinks));
  if (!r)
    return grpc::Status(grpc::StatusCode::NOT_FOUND, r.error().message_);

  std::cout << "Client " << context->peer()
            << " subscribed to market data for strategy " << strategy_id
            << "\n";

  // Holds the gRPC thread open until disconnected or not active anymore
  while (!context->IsCancelled() && active->load(std::memory_order_relaxed))
    std::this_thread::sleep_for(std::chrono::milliseconds(50));

  // ALWAYS clear before dying, we don't want the server to call a dead
  // ServerWriter
  owner_->handler_->ClearMarketDataStream(strategy_id);

  std::cout << "Client " << context->peer()
            << " unsubscribed from market data for strategy " << strategy_id
            << "\n";
  return grpc::Status::OK;
}

} // namespace quarcc
