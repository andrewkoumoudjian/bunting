#include <gmock/gmock.h>
#include <gtest/gtest.h>

#include <trading/core/order_manager.h>
#include <trading/interfaces/i_risk_check.h>

#include "helpers/proto_builders.h"
#include "mocks/mock_execution_gateway.h"
#include "mocks/mock_journal.h"
#include "mocks/mock_order_store.h"

using namespace testing;

namespace quarcc {

struct OrderManagerFixture : public Test {
  // Raw pointers kept for EXPECT_CALL ownership transferred to manager
  MockExecutionGateway *gw{};
  MockJournal *journal{};
  MockOrderStore *store{};

  std::unique_ptr<OrderManager> manager;

  void SetUp() override {
    static int id{};
    auto gw_owned = std::make_unique<NiceMock<MockExecutionGateway>>();
    auto jn_owned = std::make_unique<NiceMock<MockJournal>>();
    auto os_owned = std::make_unique<NiceMock<MockOrderStore>>();

    gw = gw_owned.get();
    journal = jn_owned.get();
    store = os_owned.get();

    manager = OrderManager::create_order_manager(
        std::format("strategy_#{}", id), std::make_unique<PositionKeeper>(),
        std::move(gw_owned), std::move(jn_owned), std::move(os_owned),
        std::make_unique<RiskManager>());
  }
};

TEST_F(OrderManagerFixture, SubmitSignalHappyPath) {
  // All store calls succeed; gateway returns a broker id
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_broker_id(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_)).WillByDefault(Return(std::string{"BROKER_1"}));

  auto result = manager->process_signal(test::make_signal());

  ASSERT_TRUE(result.has_value());
  EXPECT_FALSE(result->empty());
}

TEST_F(OrderManagerFixture, SubmitSignalGatewayReject) {
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_))
      .WillByDefault(Return(
          std::unexpected(Error{"Rejected by broker", ErrorType::Error})));

  // Status must be set to REJECTED on gateway failure
  EXPECT_CALL(*store, update_order_status(_, OrderStatus::REJECTED));

  auto result = manager->process_signal(test::make_signal());
  EXPECT_FALSE(result.has_value());
}

TEST_F(OrderManagerFixture, SubmitSignalStoreFailureIsReturned) {
  ON_CALL(*store, store_order(_))
      .WillByDefault(
          Return(std::unexpected(Error{"DB error", ErrorType::Error})));

  auto result = manager->process_signal(test::make_signal());
  EXPECT_FALSE(result.has_value());
}

TEST_F(OrderManagerFixture, CancelSignalHappyPath) {
  // Pre-populate the mapper so the cancel can resolve the broker id
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_broker_id(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_)).WillByDefault(Return(std::string{"BROKER_C1"}));

  auto submit = manager->process_signal(test::make_signal());
  ASSERT_TRUE(submit.has_value());
  const std::string local_id = *submit;

  ON_CALL(*gw, cancel_order(_)).WillByDefault(Return(std::monostate{}));
  EXPECT_CALL(*store, update_order_status(local_id, OrderStatus::CANCELLED));

  v1::CancelSignal cancel;
  cancel.set_strategy_id("TEST");
  cancel.set_order_id(local_id);

  auto result = manager->process_signal(cancel);
  EXPECT_TRUE(result.has_value());
}

TEST_F(OrderManagerFixture, CancelSignalUnknownOrderReturnsError) {
  v1::CancelSignal cancel;
  cancel.set_strategy_id("TEST");
  cancel.set_order_id("NONEXISTENT");

  auto result = manager->process_signal(cancel);
  EXPECT_FALSE(result.has_value());
}

TEST_F(OrderManagerFixture, ReplaceSignalHappyPath) {
  // First submit an order to get a local_id in the mapper
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_broker_id(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_)).WillByDefault(Return(std::string{"BROKER_R1"}));

  auto submit = manager->process_signal(test::make_signal());
  ASSERT_TRUE(submit.has_value());
  const std::string old_local_id = *submit;

  ON_CALL(*gw, replace_order(_, _))
      .WillByDefault(Return(std::string{"BROKER_R2"}));

  v1::ReplaceSignal replace;
  replace.set_strategy_id("TEST");
  replace.set_symbol("AAPL");
  replace.set_side(v1::Side::BUY);
  replace.set_target_quantity(20.0);
  replace.set_order_id(old_local_id);

  auto result = manager->process_signal(replace);
  ASSERT_TRUE(result.has_value());
  EXPECT_NE(*result, old_local_id); // new local id must be different
}

TEST_F(OrderManagerFixture, ReplaceSignalUnknownOrderReturnsError) {
  v1::ReplaceSignal replace;
  replace.set_strategy_id("TEST");
  replace.set_order_id("NONEXISTENT");

  auto result = manager->process_signal(replace);
  EXPECT_FALSE(result.has_value());
}

TEST_F(OrderManagerFixture, ProcessFillsFullyFilledOrder) {
  // Set up a submitted order and inject it via the mapper (submit path)
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_broker_id(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_)).WillByDefault(Return(std::string{"BROKER_F1"}));

  auto submit = manager->process_signal(
      test::make_signal("TEST", "AAPL", v1::Side::BUY, 10.0));
  ASSERT_TRUE(submit.has_value());
  const std::string local_id = *submit;

  // Stored order to compare original qty
  auto stored = test::make_stored_order(local_id, "AAPL", v1::Side::BUY, 10.0,
                                        OrderStatus::SUBMITTED, "BROKER_F1");
  ON_CALL(*store, get_order(local_id)).WillByDefault(Return(stored));
  ON_CALL(*store, update_fill_info(_, _, _))
      .WillByDefault(Return(std::monostate{}));

  // Fully-filled order must get FILLED status
  EXPECT_CALL(*store, update_order_status(local_id, OrderStatus::FILLED));

  auto fill = test::make_fill("BROKER_F1", "AAPL", v1::Side::BUY, 10.0, 150.0);
  gw->trigger_fill(fill);
  manager->wait_idle(); // fill is dispatched async; sync before asserting

  // Position should now reflect the fill
  auto pos = manager->get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 10.0);
}

TEST_F(OrderManagerFixture, ProcessFillsPartialFill) {
  ON_CALL(*store, store_order(_)).WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_broker_id(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*store, update_order_status(_, _))
      .WillByDefault(Return(std::monostate{}));
  ON_CALL(*gw, submit_order(_)).WillByDefault(Return(std::string{"BROKER_P1"}));

  auto submit = manager->process_signal(
      test::make_signal("TEST", "AAPL", v1::Side::BUY, 10.0));
  ASSERT_TRUE(submit.has_value());
  const std::string local_id = *submit;

  auto stored = test::make_stored_order(local_id, "AAPL", v1::Side::BUY, 10.0,
                                        OrderStatus::SUBMITTED, "BROKER_P1");
  ON_CALL(*store, get_order(local_id)).WillByDefault(Return(stored));
  ON_CALL(*store, update_fill_info(_, _, _))
      .WillByDefault(Return(std::monostate{}));

  // Partial fill -> PARTIALLY_FILLED, NOT FILLED
  EXPECT_CALL(*store,
              update_order_status(local_id, OrderStatus::PARTIALLY_FILLED));

  auto fill = test::make_fill("BROKER_P1", "AAPL", v1::Side::BUY, 5.0, 150.0);
  gw->trigger_fill(fill);
  manager->wait_idle(); // sync before mock expectations are verified
}

TEST_F(OrderManagerFixture, ProcessFillsUnknownBrokerIDIsSkipped) {
  // Fill arrives for a broker id that is not in the mapper, must not crash
  auto fill = test::make_fill("GHOST_BROKER");
  EXPECT_NO_THROW(gw->trigger_fill(fill));
  manager->wait_idle();
}

TEST_F(OrderManagerFixture, CancelAllCancelsEveryOpenOrder) {
  auto o1 = test::make_stored_order("L1", "AAPL", v1::Side::BUY, 5.0,
                                    OrderStatus::SUBMITTED, "B1");
  auto o2 = test::make_stored_order("L2", "MSFT", v1::Side::SELL, 3.0,
                                    OrderStatus::SUBMITTED, "B2");
  ON_CALL(*store, get_open_orders()).WillByDefault(Return(std::vector{o1, o2}));
  ON_CALL(*gw, cancel_order(_)).WillByDefault(Return(std::monostate{}));

  EXPECT_CALL(*store, update_order_status("L1", OrderStatus::CANCELLED));
  EXPECT_CALL(*store, update_order_status("L2", OrderStatus::CANCELLED));

  manager->cancel_all("emergency", "risk_system");
}

} // namespace quarcc
