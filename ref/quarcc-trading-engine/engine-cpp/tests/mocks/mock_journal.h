#pragma once

#include <gmock/gmock.h>
#include <trading/interfaces/i_journal.h>

namespace quarcc {

class MockJournal : public IJournal {
public:
  MOCK_METHOD(void, log,
              (Event event, const std::string &data,
               const std::string &correlation_id),
              (override));
  MOCK_METHOD(std::vector<LogEntry>, get_history,
              (Timestamp from, Timestamp to, std::optional<Event> event_filter),
              (override));
  MOCK_METHOD(std::vector<LogEntry>, get_order_history,
              (const std::string &order_id), (override));
  MOCK_METHOD(void, flush, (), (override));
};

} // namespace quarcc
