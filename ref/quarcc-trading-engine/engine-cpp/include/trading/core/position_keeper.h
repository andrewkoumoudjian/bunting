#pragma once

#include "common.pb.h"
#include "execution_service.pb.h"

#include <trading/utils/order_id_generator.h>
#include <trading/utils/result.h>

#include <shared_mutex>
#include <unordered_map>

namespace quarcc {

class PositionKeeper {
public:
  // Called by OrderManager::process_fills() whenever a fill arrives from the
  // gateway. Updates the in-memory position with a signed-quantity weighted-
  // average price calculation.
  void on_fill(const std::string &symbol, double fill_qty, double fill_price,
               v1::Side side);

  Result<v1::Position> get_position(const std::string &symbol) const;
  v1::PositionList get_all_positions() const;

private:
  struct Position {
    std::string symbol;
    double quantity = 0.0;
    double avgPrice = 0.0;
    double rPnL = 0.0;
  };

  mutable std::shared_mutex mutex_;
  std::unordered_map<std::string, Position> positions_;
};

}; // namespace quarcc
