#pragma once

#include "order.pb.h"
#include <optional>
#include <trading/utils/result.h>
#include <vector>

namespace quarcc {

enum class OrderStatus : std::uint8_t {
  PENDING_SUBMISSION = 0,
  SUBMITTED = 1,
  ACCEPTED = 2,
  PARTIALLY_FILLED = 3,
  FILLED = 4,
  CANCELLED = 5,
  REPLACED = 6,
  REJECTED = 7,
  EXPIRED = 8
};

inline const char *order_status_to_string(OrderStatus status) {
  switch (status) {
  case OrderStatus::PENDING_SUBMISSION:
    return "PENDING_SUBMISSION";
  case OrderStatus::SUBMITTED:
    return "SUBMITTED";
  case OrderStatus::ACCEPTED:
    return "ACCEPTED";
  case OrderStatus::PARTIALLY_FILLED:
    return "PARTIALLY_FILLED";
  case OrderStatus::FILLED:
    return "FILLED";
  case OrderStatus::CANCELLED:
    return "CANCELLED";
  case OrderStatus::REPLACED:
    return "REPLACED";
  case OrderStatus::REJECTED:
    return "REJECTED";
  case OrderStatus::EXPIRED:
    return "EXPIRED";
  default:
    return "UNKNOWN";
  }
}

struct StoredOrder {
  v1::Order order;
  OrderStatus status;
  std::string local_id;
  std::optional<std::string> broker_id;
  std::string created_at;
  std::optional<std::string> updated_at;
  double filled_quantity = 0.0;
  double avg_fill_price = 0.0;
};

class IOrderStore {
public:
  virtual ~IOrderStore() = default;

  virtual Result<std::monostate> store_order(const StoredOrder &order) = 0;
  virtual Result<std::monostate>
  update_order_status(const std::string &local_id, OrderStatus new_status) = 0;
  virtual Result<std::monostate>
  update_broker_id(const std::string &local_id,
                   const std::string &broker_id) = 0;
  virtual Result<std::monostate> update_fill_info(const std::string &local_id,
                                                  double filled_quantity,
                                                  double avg_price) = 0;
  virtual Result<StoredOrder> get_order(const std::string &local_id) = 0;
  virtual std::vector<StoredOrder> get_open_orders() = 0;
  virtual std::vector<StoredOrder> get_orders_by_status(OrderStatus status) = 0;
};

} // namespace quarcc
