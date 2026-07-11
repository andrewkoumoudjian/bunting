#pragma once

#include <mutex>
#include <sqlite3.h>
#include <trading/interfaces/i_order_store.h>

namespace quarcc {

class SQLiteOrderStore : public IOrderStore {
public:
  explicit SQLiteOrderStore(const std::string &db_path);
  ~SQLiteOrderStore() override;

  SQLiteOrderStore(const SQLiteOrderStore &) = delete;
  SQLiteOrderStore &operator=(const SQLiteOrderStore &) = delete;

  Result<std::monostate> store_order(const StoredOrder &order) override;
  Result<std::monostate> update_order_status(const std::string &local_id,
                                             OrderStatus new_status) override;
  Result<std::monostate>
  update_broker_id(const std::string &local_id,
                   const std::string &broker_id) override;
  Result<std::monostate> update_fill_info(const std::string &local_id,
                                          double filled_quantity,
                                          double avg_price) override;
  Result<StoredOrder> get_order(const std::string &local_id) override;
  std::vector<StoredOrder> get_open_orders() override;
  std::vector<StoredOrder> get_orders_by_status(OrderStatus status) override;

private:
  void create_schema();
  StoredOrder parse_order(sqlite3_stmt *stmt);

  sqlite3 *db_ = nullptr;
  mutable std::mutex mutex_;
};

} // namespace quarcc
