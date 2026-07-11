#pragma once

#include <gmock/gmock.h>
#include <trading/interfaces/i_order_store.h>

namespace quarcc {

class MockOrderStore : public IOrderStore {
public:
  MOCK_METHOD(Result<std::monostate>, store_order, (const StoredOrder &order),
              (override));
  MOCK_METHOD(Result<std::monostate>, update_order_status,
              (const std::string &local_id, OrderStatus new_status),
              (override));
  MOCK_METHOD(Result<std::monostate>, update_broker_id,
              (const std::string &local_id, const std::string &broker_id),
              (override));
  MOCK_METHOD(Result<std::monostate>, update_fill_info,
              (const std::string &local_id, double filled_quantity,
               double avg_price),
              (override));
  MOCK_METHOD(Result<StoredOrder>, get_order, (const std::string &local_id),
              (override));
  MOCK_METHOD(std::vector<StoredOrder>, get_open_orders, (), (override));
  MOCK_METHOD(std::vector<StoredOrder>, get_orders_by_status,
              (OrderStatus status), (override));
};

} // namespace quarcc
