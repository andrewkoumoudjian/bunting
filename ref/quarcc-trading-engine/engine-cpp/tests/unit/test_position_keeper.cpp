#include <gtest/gtest.h>
#include <trading/core/position_keeper.h>

namespace quarcc {

// ---- helpers ----

static void buy(PositionKeeper &pk, const std::string &sym, double qty,
                double price) {
  pk.on_fill(sym, qty, price, v1::Side::BUY);
}

static void sell(PositionKeeper &pk, const std::string &sym, double qty,
                 double price) {
  pk.on_fill(sym, qty, price, v1::Side::SELL);
}

// ---- tests ----

TEST(PositionKeeper, FreshKeeperHasNoPosition) {
  PositionKeeper pk;
  EXPECT_FALSE(pk.get_position("AAPL").has_value());
  EXPECT_EQ(pk.get_all_positions().positions_size(), 0);
}

TEST(PositionKeeper, SingleBuyOpensPosition) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 150.0);

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 10.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 150.0);
}

TEST(PositionKeeper, SecondBuyAccumulatesWeightedAvg) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 100.0); // avg = 100
  buy(pk, "AAPL", 10.0, 200.0); // avg = (10*100 + 10*200)/20 = 150

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 20.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 150.0);
}

TEST(PositionKeeper, PartialSellReducesQtyKeepsAvgPrice) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 150.0);
  sell(pk, "AAPL", 4.0, 160.0); // reducing; avg price must stay 150

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 6.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 150.0);
}

TEST(PositionKeeper, FullSellFlattensPosition) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 150.0);
  sell(pk, "AAPL", 10.0, 160.0); // position goes to zero

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 0.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 0.0);
}

TEST(PositionKeeper, SellBeyondFlatFlipsToShort) {
  PositionKeeper pk;
  buy(pk, "AAPL", 5.0, 100.0);
  sell(pk, "AAPL", 10.0, 120.0); // flips: net = -5, avg = fill price

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), -5.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 120.0);
}

TEST(PositionKeeper, ZeroFillQtyIsIgnored) {
  PositionKeeper pk;
  pk.on_fill("AAPL", 0.0, 150.0, v1::Side::BUY);
  EXPECT_FALSE(pk.get_position("AAPL").has_value());
}

TEST(PositionKeeper, ZeroFillPriceUpdatesQtyOnly) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 150.0);
  // Paper gateway may omit price; qty should update, avg must not change
  pk.on_fill("AAPL", 5.0, 0.0, v1::Side::BUY);

  auto pos = pk.get_position("AAPL");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), 15.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 150.0); // unchanged
}

TEST(PositionKeeper, GetAllPositionsReturnsAllSymbols) {
  PositionKeeper pk;
  buy(pk, "AAPL", 10.0, 150.0);
  buy(pk, "MSFT", 5.0, 300.0);

  auto all = pk.get_all_positions();
  EXPECT_EQ(all.positions_size(), 2);
}

TEST(PositionKeeper, ShortPositionFromScratch) {
  PositionKeeper pk;
  sell(pk, "TSLA", 3.0, 200.0);

  auto pos = pk.get_position("TSLA");
  ASSERT_TRUE(pos.has_value());
  EXPECT_DOUBLE_EQ(pos->quantity(), -3.0);
  EXPECT_DOUBLE_EQ(pos->avg_price(), 200.0);
}

} // namespace quarcc
