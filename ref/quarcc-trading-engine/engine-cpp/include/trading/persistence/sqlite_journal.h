#pragma once

#include <mutex>
#include <sqlite3.h>
#include <trading/interfaces/i_journal.h>

namespace quarcc {

class SQLiteJournal : public IJournal {
public:
  explicit SQLiteJournal(const std::string &db_path);
  ~SQLiteJournal() override;

  SQLiteJournal(const SQLiteJournal &) = delete;
  SQLiteJournal &operator=(const SQLiteJournal &) = delete;

  void log(Event event, const std::string &data,
           const std::string &correlation_id = "") override;

  std::vector<LogEntry>
  get_history(Timestamp from, Timestamp to,
              std::optional<Event> event_filter = std::nullopt) override;

  std::vector<LogEntry> get_order_history(const std::string &order_id) override;

  void flush() override;

private:
  void create_schema();

  sqlite3 *db_ = nullptr;
  mutable std::mutex mutex_;
};

} // namespace quarcc
