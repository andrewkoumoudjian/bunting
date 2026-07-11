#pragma once

#include "execution_service.grpc.pb.h"
#include "execution_service.pb.h"

#include <grpcpp/grpcpp.h>
#include <memory>
#include <string>

#include <trading/interfaces/i_execution_service_handler.h>

namespace quarcc {

class gRPCServer {
public:
  gRPCServer(std::string server_address, IExecutionServiceHandler &handler);

  void start();
  void wait();
  void shutdown();

private:
  class ExecutionServiceImpl final : public v1::ExecutionService::Service {
  public:
    explicit ExecutionServiceImpl(gRPCServer *owner);

    grpc::Status SubmitSignal(grpc::ServerContext *context,
                              const v1::StrategySignal *request,
                              v1::SubmitSignalResponse *response) override;

    grpc::Status CancelOrder(grpc::ServerContext *context,
                             const v1::CancelSignal *request,
                             v1::CancelOrderResponse *response) override;

    grpc::Status ReplaceOrder(grpc::ServerContext *context,
                              const v1::ReplaceSignal *request,
                              v1::ReplaceOrderResponse *response) override;

    grpc::Status StreamSignals(
        grpc::ServerContext *context,
        grpc::ServerReaderWriter<v1::SubmitSignalResponse, v1::StrategySignal>
            *stream) override;

    grpc::Status GetPosition(grpc::ServerContext *context,
                             const v1::GetPositionRequest *request,
                             v1::Position *response) override;

    grpc::Status GetAllPositions(grpc::ServerContext *context,
                                 const v1::Empty *request,
                                 v1::PositionList *response) override;

    grpc::Status ActivateKillSwitch(grpc::ServerContext *context,
                                    const v1::KillSwitchRequest *request,
                                    v1::Empty *response) override;

    grpc::Status
    StreamMarketData(grpc::ServerContext *context,
                     const v1::SubscribeMarketDataRequest *request,
                     grpc::ServerWriter<v1::MarketDataEvent> *writer) override;

  private:
    gRPCServer *owner_ = nullptr;
  };

private:
  friend class ExecutionServiceImpl;

  std::string server_address_;
  IExecutionServiceHandler *handler_ = nullptr;
  std::unique_ptr<ExecutionServiceImpl> service_;
  std::unique_ptr<grpc::Server> server_;
};

} // namespace quarcc
