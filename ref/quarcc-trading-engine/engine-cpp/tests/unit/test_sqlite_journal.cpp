// Integration tests for SQLiteJournal using an in-memory SQLite database.
// The special path ":memory:" is supported by SQLite and creates a temporary
// database that is discarded when the connection closes.

#include <gtest/gtest.h>
#include <trading/persistence/sqlite_journal.h>

#include <chrono>
#include <thread>

namespace quarcc {

struct JournalFixture : public testing::Test {
  SQLiteJournal journal{":memory:"};
};

TEST_F(JournalFixture, LogAndRetrieveSingleEntry) {
  journal.log(Event::ORDER_CREATED, "test data", "ORDER_1");

  auto from = LogEntry::now() - std::chrono::seconds{5};
  auto to = LogEntry::now() + std::chrono::seconds{5};
  auto entries = journal.get_history(from, to);

  ASSERT_EQ(entries.size(), 1u);
  EXPECT_EQ(entries[0].event_type, Event::ORDER_CREATED);
  EXPECT_EQ(entries[0].data, "test data");
  EXPECT_EQ(entries[0].correlation_id, "ORDER_1");
}

TEST_F(JournalFixture, GetHistoryWithEventFilterReturnsOnlyMatchingEvents) {
  journal.log(Event::ORDER_CREATED, "created", "ORD_A");
  journal.log(Event::ORDER_SUBMITTED, "submitted", "ORD_A");
  journal.log(Event::ORDER_FILLED, "filled", "ORD_A");

  auto from = LogEntry::now() - std::chrono::seconds{5};
  auto to = LogEntry::now() + std::chrono::seconds{5};
  auto entries = journal.get_history(from, to, Event::ORDER_SUBMITTED);

  ASSERT_EQ(entries.size(), 1u);
  EXPECT_EQ(entries[0].event_type, Event::ORDER_SUBMITTED);
}

TEST_F(JournalFixture, GetOrderHistoryFiltersByCorrelationId) {
  journal.log(Event::ORDER_CREATED, "created", "ORD_X");
  journal.log(Event::ORDER_SUBMITTED, "submitted", "ORD_X");
  journal.log(Event::ORDER_CREATED, "other order", "ORD_Y");

  auto entries = journal.get_order_history("ORD_X");

  ASSERT_EQ(entries.size(), 2u);
  for (const auto &e : entries)
    EXPECT_EQ(e.correlation_id, "ORD_X");
}

TEST_F(JournalFixture, GetOrderHistoryEmptyForUnknownId) {
  auto entries = journal.get_order_history("NO_SUCH_ORDER");
  EXPECT_TRUE(entries.empty());
}

TEST_F(JournalFixture, FlushDoesNotThrow) {
  journal.log(Event::SYSTEM_STARTED, "startup", "");
  EXPECT_NO_THROW(journal.flush());
}

TEST_F(JournalFixture, MultipleEntriesAreOrderedByInsertionId) {
  journal.log(Event::ORDER_CREATED, "1", "ORD_SEQ");
  // Small sleep to guarantee distinct timestamps (journal has UNIQUE constraint
  // on timestamp+correlation_id+event_type)
  std::this_thread::sleep_for(std::chrono::milliseconds{10});
  journal.log(Event::ORDER_SUBMITTED, "2", "ORD_SEQ");
  std::this_thread::sleep_for(std::chrono::milliseconds{10});
  journal.log(Event::ORDER_FILLED, "3", "ORD_SEQ");

  auto entries = journal.get_order_history("ORD_SEQ");
  ASSERT_EQ(entries.size(), 3u);
  EXPECT_EQ(entries[0].event_type, Event::ORDER_CREATED);
  EXPECT_EQ(entries[1].event_type, Event::ORDER_SUBMITTED);
  EXPECT_EQ(entries[2].event_type, Event::ORDER_FILLED);
}

TEST_F(JournalFixture, LogWithoutCorrelationIdSucceeds) {
  EXPECT_NO_THROW(journal.log(Event::SYSTEM_STARTED, "no correlation id"));

  auto from = LogEntry::now() - std::chrono::seconds{5};
  auto to = LogEntry::now() + std::chrono::seconds{5};
  auto entries = journal.get_history(from, to);
  EXPECT_FALSE(entries.empty());
}

} // namespace quarcc
