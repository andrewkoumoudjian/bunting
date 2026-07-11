// Integration tests for SQLiteOrderStore using an in-memory SQLite database.

#include <gtest/gtest.h>
#include <trading/persistence/sqlite_order_store.h>

#include "helpers/proto_builders.h"

namespace quarcc {

struct OrderStoreFixture : public testing::Test {
  SQLiteOrderStore store{":memory:"};
};

TEST_F(OrderStoreFixture, StoreAndRetrieveOrder) {
  auto stored = test::make_stored_order("L1", "AAPL", v1::Side::BUY, 10.0);
  auto result = store.store_order(stored);
  ASSERT_TRUE(result.has_value());

  auto fetched = store.get_order("L1");
  ASSERT_TRUE(fetched.has_value());
  EXPECT_EQ(fetched->local_id, "L1");
  EXPECT_EQ(fetched->order.symbol(), "AAPL");
  EXPECT_DOUBLE_EQ(fetched->order.quantity(), 10.0);
}

TEST_F(OrderStoreFixture, GetOrderUnknownIdReturnsError) {
  auto result = store.get_order("NO_SUCH_ID");
  EXPECT_FALSE(result.has_value());
}

TEST_F(OrderStoreFixture, UpdateOrderStatusChangesStatus) {
  store.store_order(test::make_stored_order("L2"));

  auto result = store.update_order_status("L2", OrderStatus::FILLED);
  ASSERT_TRUE(result.has_value());

  auto fetched = store.get_order("L2");
  ASSERT_TRUE(fetched.has_value());
  EXPECT_EQ(fetched->status, OrderStatus::FILLED);
}

TEST_F(OrderStoreFixture, UpdateBrokerIdSetsBrokerId) {
  store.store_order(
      test::make_stored_order("L3", "AAPL", v1::Side::BUY, 10.0,
                              OrderStatus::PENDING_SUBMISSION, std::nullopt));

  auto result = store.update_broker_id("L3", "BROKER_99");
  ASSERT_TRUE(result.has_value());

  auto fetched = store.get_order("L3");
  ASSERT_TRUE(fetched.has_value());
  ASSERT_TRUE(fetched->broker_id.has_value());
  EXPECT_EQ(*fetched->broker_id, "BROKER_99");
}

TEST_F(OrderStoreFixture, UpdateFillInfoSetsFillFields) {
  store.store_order(test::make_stored_order("L4"));

  auto result = store.update_fill_info("L4", 7.5, 152.25);
  ASSERT_TRUE(result.has_value());

  auto fetched = store.get_order("L4");
  ASSERT_TRUE(fetched.has_value());
  EXPECT_DOUBLE_EQ(fetched->filled_quantity, 7.5);
  EXPECT_DOUBLE_EQ(fetched->avg_fill_price, 152.25);
}

TEST_F(OrderStoreFixture, GetOpenOrdersReturnsOnlyNonTerminalOrders) {
  store.store_order(test::make_stored_order("OPEN_1", "AAPL", v1::Side::BUY,
                                            5.0, OrderStatus::SUBMITTED));
  store.store_order(test::make_stored_order("OPEN_2", "MSFT", v1::Side::BUY,
                                            5.0, OrderStatus::PARTIALLY_FILLED));
  store.store_order(test::make_stored_order("CLOSED", "TSLA", v1::Side::SELL,
                                            5.0, OrderStatus::FILLED));

  auto open = store.get_open_orders();

  // FILLED order must not appear in open orders
  for (const auto &o : open)
    EXPECT_NE(o.local_id, "CLOSED");

  // Both open orders must be present
  bool found_open1 = false, found_open2 = false;
  for (const auto &o : open) {
    if (o.local_id == "OPEN_1") found_open1 = true;
    if (o.local_id == "OPEN_2") found_open2 = true;
  }
  EXPECT_TRUE(found_open1);
  EXPECT_TRUE(found_open2);
}

TEST_F(OrderStoreFixture, GetOrdersByStatusFiltersCorrectly) {
  store.store_order(test::make_stored_order("S1", "AAPL", v1::Side::BUY, 5.0,
                                            OrderStatus::CANCELLED));
  store.store_order(test::make_stored_order("S2", "MSFT", v1::Side::BUY, 5.0,
                                            OrderStatus::SUBMITTED));

  auto cancelled = store.get_orders_by_status(OrderStatus::CANCELLED);
  ASSERT_EQ(cancelled.size(), 1u);
  EXPECT_EQ(cancelled[0].local_id, "S1");

  auto submitted = store.get_orders_by_status(OrderStatus::SUBMITTED);
  ASSERT_EQ(submitted.size(), 1u);
  EXPECT_EQ(submitted[0].local_id, "S2");
}

TEST_F(OrderStoreFixture, GetOpenOrdersEmptyOnFreshStore) {
  auto open = store.get_open_orders();
  EXPECT_TRUE(open.empty());
}

} // namespace quarcc
