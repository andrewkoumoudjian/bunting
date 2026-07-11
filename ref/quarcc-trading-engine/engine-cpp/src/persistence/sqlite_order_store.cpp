#include <iostream>
#include <stdexcept>
#include <trading/persistence/sqlite_order_store.h>

namespace quarcc {

SQLiteOrderStore::SQLiteOrderStore(const std::string &db_path) {
  int rc = sqlite3_open(db_path.c_str(), &db_);
  if (rc != SQLITE_OK) [[unlikely]] {
    std::string error = sqlite3_errmsg(db_);
    throw std::runtime_error("Failed to open order store database: " + error);
  }

  sqlite3_exec(db_, "PRAGMA journal_mode=WAL;", nullptr, nullptr, nullptr);
  sqlite3_exec(db_, "PRAGMA synchronous=NORMAL;", nullptr, nullptr, nullptr);
  sqlite3_exec(db_, "PRAGMA cache_size=-8000;", nullptr, nullptr, nullptr);  // 8MB cache

  create_schema();
}

SQLiteOrderStore::~SQLiteOrderStore() {
  if (db_) {
    sqlite3_close(db_);
  }
}

void SQLiteOrderStore::create_schema() {
  const char *sql = R"(
    CREATE TABLE IF NOT EXISTS orders (
      local_id TEXT PRIMARY KEY,
      broker_id TEXT UNIQUE,
      symbol TEXT NOT NULL,
      side INTEGER NOT NULL,
      quantity REAL NOT NULL,
      price REAL,
      order_type INTEGER NOT NULL,
      status INTEGER NOT NULL,
      time_in_force INTEGER NOT NULL,
      account_id TEXT NOT NULL,
      strategy_id TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT,
      filled_quantity REAL DEFAULT 0.0,
      avg_fill_price REAL DEFAULT 0.0,
      order_proto BLOB NOT NULL
    );
    
    CREATE INDEX IF NOT EXISTS idx_status ON orders(status);
    CREATE INDEX IF NOT EXISTS idx_strategy ON orders(strategy_id);
    CREATE INDEX IF NOT EXISTS idx_broker_id ON orders(broker_id);
    CREATE INDEX IF NOT EXISTS idx_created_at ON orders(created_at);
  )";

  char *err_msg = nullptr;
  int rc = sqlite3_exec(db_, sql, nullptr, nullptr, &err_msg);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::string error = err_msg;
    sqlite3_free(err_msg);
    throw std::runtime_error("Failed to create order store schema: " + error);
  }
}

Result<std::monostate>
SQLiteOrderStore::store_order(const StoredOrder &stored_order) {
  std::lock_guard lock(mutex_);

  const char *sql = R"(
    INSERT INTO orders (
      local_id, broker_id, symbol, side, quantity, price, 
      order_type, status, time_in_force, account_id, strategy_id,
      created_at, filled_quantity, avg_fill_price, order_proto
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    return std::unexpected(Error{"Failed to prepare insert statement: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  const auto &order = stored_order.order;

  // Bind values
  sqlite3_bind_text(stmt, 1, stored_order.local_id.c_str(), -1,
                    SQLITE_TRANSIENT);

  if (stored_order.broker_id) {
    sqlite3_bind_text(stmt, 2, stored_order.broker_id->c_str(), -1,
                      SQLITE_TRANSIENT);
  } else {
    sqlite3_bind_null(stmt, 2);
  }

  sqlite3_bind_text(stmt, 3, order.symbol().c_str(), -1, SQLITE_TRANSIENT);
  sqlite3_bind_int(stmt, 4, static_cast<int>(order.side()));
  sqlite3_bind_double(stmt, 5, order.quantity());
  sqlite3_bind_double(stmt, 6, order.price());
  sqlite3_bind_int(stmt, 7, static_cast<int>(order.type()));
  sqlite3_bind_int(stmt, 8, static_cast<int>(stored_order.status));
  sqlite3_bind_int(stmt, 9, static_cast<int>(order.time_in_force()));
  sqlite3_bind_text(stmt, 10, order.account_id().c_str(), -1, SQLITE_TRANSIENT);
  sqlite3_bind_text(stmt, 11, order.strategy_id().c_str(), -1,
                    SQLITE_TRANSIENT);
  sqlite3_bind_text(stmt, 12, stored_order.created_at.c_str(), -1,
                    SQLITE_TRANSIENT);
  sqlite3_bind_double(stmt, 13, stored_order.filled_quantity);
  sqlite3_bind_double(stmt, 14, stored_order.avg_fill_price);

  std::string serialized;
  order.SerializeToString(&serialized);
  sqlite3_bind_blob(stmt, 15, serialized.data(), serialized.size(),
                    SQLITE_TRANSIENT);

  rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);

  if (rc != SQLITE_DONE) [[unlikely]] {
    return std::unexpected(
        Error{"Failed to insert order: " + std::string(sqlite3_errmsg(db_)),
              ErrorType::Error});
  }

  return std::monostate{};
}

Result<std::monostate>
SQLiteOrderStore::update_order_status(const std::string &local_id,
                                      OrderStatus new_status) {

  std::lock_guard lock(mutex_);

  const char *sql = R"(
    UPDATE orders 
    SET status = ?, updated_at = datetime('now')
    WHERE local_id = ?
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    return std::unexpected(Error{"Failed to prepare update statement: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  sqlite3_bind_int(stmt, 1, static_cast<int>(new_status));
  sqlite3_bind_text(stmt, 2, local_id.c_str(), -1, SQLITE_TRANSIENT);

  rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);

  if (rc != SQLITE_DONE) [[unlikely]] {
    return std::unexpected(Error{"Failed to update order status: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  return std::monostate{};
}

Result<std::monostate>
SQLiteOrderStore::update_broker_id(const std::string &local_id,
                                   const std::string &broker_id) {

  std::lock_guard lock(mutex_);

  const char *sql = R"(
    UPDATE orders 
    SET broker_id = ?, updated_at = datetime('now')
    WHERE local_id = ?
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    return std::unexpected(Error{"Failed to prepare update statement: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  sqlite3_bind_text(stmt, 1, broker_id.c_str(), -1, SQLITE_TRANSIENT);
  sqlite3_bind_text(stmt, 2, local_id.c_str(), -1, SQLITE_TRANSIENT);

  rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);

  if (rc != SQLITE_DONE) [[unlikely]] {
    return std::unexpected(
        Error{"Failed to update broker ID: " + std::string(sqlite3_errmsg(db_)),
              ErrorType::Error});
  }

  return std::monostate{};
}

Result<std::monostate>
SQLiteOrderStore::update_fill_info(const std::string &local_id,
                                   double filled_quantity, double avg_price) {

  std::lock_guard lock(mutex_);

  const char *sql = R"(
    UPDATE orders
    SET
      avg_fill_price =
        CASE
          WHEN filled_quantity = 0 THEN ?
          ELSE (avg_fill_price * filled_quantity + ? * ?) /
               (filled_quantity + ?)
        END,
      filled_quantity = filled_quantity + ?,
      updated_at = datetime('now')
    WHERE local_id = ?
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    return std::unexpected(Error{"Failed to prepare update statement: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  // TODO: Verify if this is the right values for the placeholders
  sqlite3_bind_double(stmt, 1, avg_price);
  sqlite3_bind_double(stmt, 2, avg_price);
  sqlite3_bind_double(stmt, 3, filled_quantity);
  sqlite3_bind_double(stmt, 4, filled_quantity);
  sqlite3_bind_double(stmt, 5, filled_quantity);
  sqlite3_bind_text(stmt, 6, local_id.c_str(), -1, SQLITE_TRANSIENT);

  rc = sqlite3_step(stmt);
  sqlite3_finalize(stmt);

  if (rc != SQLITE_DONE) {
    return std::unexpected(
        Error{"Failed to update fill info: " + std::string(sqlite3_errmsg(db_)),
              ErrorType::Error});
  }

  return std::monostate{};
}

StoredOrder SQLiteOrderStore::parse_order(sqlite3_stmt *stmt) {
  StoredOrder stored;

  stored.local_id =
      reinterpret_cast<const char *>(sqlite3_column_text(stmt, 0));

  const char *broker_id =
      reinterpret_cast<const char *>(sqlite3_column_text(stmt, 1));
  if (broker_id) {
    stored.broker_id = broker_id;
  }

  stored.status = static_cast<OrderStatus>(sqlite3_column_int(stmt, 2));
  stored.created_at =
      reinterpret_cast<const char *>(sqlite3_column_text(stmt, 3));

  const char *updated_at =
      reinterpret_cast<const char *>(sqlite3_column_text(stmt, 4));
  if (updated_at) {
    stored.updated_at = updated_at;
  }

  stored.filled_quantity = sqlite3_column_double(stmt, 5);
  stored.avg_fill_price = sqlite3_column_double(stmt, 6);

  const void *blob = sqlite3_column_blob(stmt, 7);
  int size = sqlite3_column_bytes(stmt, 7);
  stored.order.ParseFromArray(blob, size);

  return stored;
}

Result<StoredOrder> SQLiteOrderStore::get_order(const std::string &local_id) {
  std::lock_guard lock(mutex_);

  const char *sql = R"(
    SELECT local_id, broker_id, status, created_at, updated_at, 
           filled_quantity, avg_fill_price, order_proto
    FROM orders 
    WHERE local_id = ?
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    return std::unexpected(Error{"Failed to prepare select statement: " +
                                     std::string(sqlite3_errmsg(db_)),
                                 ErrorType::Error});
  }

  sqlite3_bind_text(stmt, 1, local_id.c_str(), -1, SQLITE_TRANSIENT);

  rc = sqlite3_step(stmt);

  if (rc == SQLITE_ROW) [[unlikely]] {
    StoredOrder order = parse_order(stmt);
    sqlite3_finalize(stmt);
    return order;
  }

  sqlite3_finalize(stmt);
  return std::unexpected(
      Error{"Order not found: " + local_id, ErrorType::Error});
}

std::vector<StoredOrder> SQLiteOrderStore::get_open_orders() {
  std::lock_guard lock(mutex_);
  std::vector<StoredOrder> orders;

  const char *sql = R"(
    SELECT local_id, broker_id, status, created_at, updated_at,
           filled_quantity, avg_fill_price, order_proto
    FROM orders 
    WHERE status IN (?, ?, ?, ?)
    ORDER BY created_at ASC
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::cerr << "Failed to prepare query: " << sqlite3_errmsg(db_)
              << std::endl;
    return orders;
  }

  sqlite3_bind_int(stmt, 1, static_cast<int>(OrderStatus::PENDING_SUBMISSION));
  sqlite3_bind_int(stmt, 2, static_cast<int>(OrderStatus::SUBMITTED));
  sqlite3_bind_int(stmt, 3, static_cast<int>(OrderStatus::ACCEPTED));
  sqlite3_bind_int(stmt, 4, static_cast<int>(OrderStatus::PARTIALLY_FILLED));

  while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
    orders.push_back(parse_order(stmt));
  }

  sqlite3_finalize(stmt);
  return orders;
}

std::vector<StoredOrder>
SQLiteOrderStore::get_orders_by_status(OrderStatus status) {
  std::lock_guard lock(mutex_);
  std::vector<StoredOrder> orders;

  const char *sql = R"(
    SELECT local_id, broker_id, status, created_at, updated_at,
           filled_quantity, avg_fill_price, order_proto
    FROM orders 
    WHERE status = ?
    ORDER BY created_at ASC
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::cerr << "Failed to prepare query: " << sqlite3_errmsg(db_)
              << std::endl;
    return orders;
  }

  sqlite3_bind_int(stmt, 1, static_cast<int>(status));

  while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
    orders.push_back(parse_order(stmt));
  }

  sqlite3_finalize(stmt);
  return orders;
}

} // namespace quarcc
