#include <iostream>
#include <stdexcept>
#include <trading/persistence/sqlite_journal.h>

namespace quarcc {

SQLiteJournal::SQLiteJournal(const std::string &db_path) {
  int rc = sqlite3_open(db_path.c_str(), &db_);
  if (rc != SQLITE_OK) [[unlikely]] {
    std::string error = sqlite3_errmsg(db_);
    throw std::runtime_error("Failed to open journal database: " + error);
  }

  sqlite3_exec(db_, "PRAGMA journal_mode=WAL;", nullptr, nullptr, nullptr);
  sqlite3_exec(db_, "PRAGMA synchronous=NORMAL;", nullptr, nullptr, nullptr);
  sqlite3_exec(db_, "PRAGMA cache_size=-8000;", nullptr, nullptr, nullptr);  // 8MB cache
  
  create_schema();
}

SQLiteJournal::~SQLiteJournal() {
  if (db_) {
    flush();
    sqlite3_close(db_);
  }
}

void SQLiteJournal::create_schema() {
  const char *sql = R"(
    CREATE TABLE IF NOT EXISTS journal (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      timestamp TEXT NOT NULL,
      event_type INTEGER NOT NULL,
      data TEXT NOT NULL,
      correlation_id TEXT,
      UNIQUE(timestamp, correlation_id, event_type)
    );
    
    CREATE INDEX IF NOT EXISTS idx_timestamp ON journal(timestamp);
    CREATE INDEX IF NOT EXISTS idx_event_type ON journal(event_type);
    CREATE INDEX IF NOT EXISTS idx_correlation_id ON journal(correlation_id);
  )";

  char *err_msg = nullptr;
  int rc = sqlite3_exec(db_, sql, nullptr, nullptr, &err_msg);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::string error = err_msg;
    sqlite3_free(err_msg);
    throw std::runtime_error("Failed to create journal schema: " + error);
  }
}

void SQLiteJournal::log(Event event, const std::string &data,
                        const std::string &correlation_id) {
  std::lock_guard lock(mutex_);

  const char *sql = R"(
    INSERT INTO journal (timestamp, event_type, data, correlation_id) 
    VALUES (?, ?, ?, ?)
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::cerr << "Failed to prepare statement: " << sqlite3_errmsg(db_)
              << std::endl;
    return;
  }

  auto now = LogEntry::now();
  std::string timestamp_str = LogEntry::timestamp_to_string(now);

  sqlite3_bind_text(stmt, 1, timestamp_str.c_str(), -1, SQLITE_TRANSIENT);
  sqlite3_bind_int(stmt, 2, static_cast<int>(event));
  sqlite3_bind_text(stmt, 3, data.c_str(), -1, SQLITE_TRANSIENT);

  if (!correlation_id.empty()) {
    sqlite3_bind_text(stmt, 4, correlation_id.c_str(), -1, SQLITE_TRANSIENT);
  } else {
    sqlite3_bind_null(stmt, 4);
  }

  // Execute
  rc = sqlite3_step(stmt);
  if (rc != SQLITE_DONE) [[unlikely]] {
    std::cerr << "Failed to insert log: " << sqlite3_errmsg(db_) << std::endl;
  }

  sqlite3_finalize(stmt);
}

std::vector<LogEntry>
SQLiteJournal::get_history(Timestamp from, Timestamp to,
                           std::optional<Event> event_filter) {

  std::lock_guard lock(mutex_);
  std::vector<LogEntry> entries;

  std::string sql = R"(
    SELECT id, timestamp, event_type, data, correlation_id 
    FROM journal 
    WHERE timestamp BETWEEN ? AND ?
  )";

  if (event_filter) {
    sql += " AND event_type = ?";
  }

  sql += " ORDER BY id ASC";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql.c_str(), -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::cerr << "Failed to prepare query: " << sqlite3_errmsg(db_)
              << std::endl;
    return entries;
  }

  std::string from_str = LogEntry::timestamp_to_string(from);
  std::string to_str = LogEntry::timestamp_to_string(to);

  sqlite3_bind_text(stmt, 1, from_str.c_str(), -1, SQLITE_TRANSIENT);
  sqlite3_bind_text(stmt, 2, to_str.c_str(), -1, SQLITE_TRANSIENT);

  if (event_filter) {
    sqlite3_bind_int(stmt, 3, static_cast<int>(*event_filter));
  }

  // Fetch results
  while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
    LogEntry entry;
    entry.id = sqlite3_column_int64(stmt, 0);

    const char *timestamp_str =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 1));
    entry.timestamp = LogEntry::string_to_timestamp(timestamp_str);

    entry.event_type = static_cast<Event>(sqlite3_column_int(stmt, 2));

    const char *data_str =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 3));
    entry.data = data_str ? data_str : "";

    const char *corr_id =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 4));
    entry.correlation_id = corr_id ? corr_id : "";

    entries.push_back(std::move(entry));
  }

  sqlite3_finalize(stmt);
  return entries;
}

std::vector<LogEntry>
SQLiteJournal::get_order_history(const std::string &order_id) {
  std::lock_guard lock(mutex_);
  std::vector<LogEntry> entries;

  const char *sql = R"(
    SELECT id, timestamp, event_type, data, correlation_id 
    FROM journal 
    WHERE correlation_id = ?
    ORDER BY id ASC
  )";

  sqlite3_stmt *stmt = nullptr;
  int rc = sqlite3_prepare_v2(db_, sql, -1, &stmt, nullptr);

  if (rc != SQLITE_OK) [[unlikely]] {
    std::cerr << "Failed to prepare query: " << sqlite3_errmsg(db_)
              << std::endl;
    return entries;
  }

  sqlite3_bind_text(stmt, 1, order_id.c_str(), -1, SQLITE_TRANSIENT);

  while ((rc = sqlite3_step(stmt)) == SQLITE_ROW) {
    LogEntry entry;
    entry.id = sqlite3_column_int64(stmt, 0);

    const char *timestamp_str =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 1));
    entry.timestamp = LogEntry::string_to_timestamp(timestamp_str);

    entry.event_type = static_cast<Event>(sqlite3_column_int(stmt, 2));

    const char *data_str =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 3));
    entry.data = data_str ? data_str : "";

    const char *corr_id =
        reinterpret_cast<const char *>(sqlite3_column_text(stmt, 4));
    entry.correlation_id = corr_id ? corr_id : "";

    entries.push_back(std::move(entry));
  }

  sqlite3_finalize(stmt);
  return entries;
}

void SQLiteJournal::flush() {
  std::lock_guard lock(mutex_);
  sqlite3_wal_checkpoint(db_, nullptr);
}

} // namespace quarcc
