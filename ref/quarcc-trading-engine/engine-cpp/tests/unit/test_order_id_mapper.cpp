#include <gtest/gtest.h>
#include <trading/utils/order_id_types.h>

namespace quarcc {

TEST(OrderIdMapper, AddMappingAndRetrieveBrokerID) {
  OrderIdMapper mapper;
  mapper.add_mapping("LOCAL_1", "BROKER_1");

  auto broker = mapper.get_broker_id("LOCAL_1");
  ASSERT_TRUE(broker.has_value());
  EXPECT_EQ(*broker, "BROKER_1");
}

TEST(OrderIdMapper, AddMappingAndRetrieveLocalID) {
  OrderIdMapper mapper;
  mapper.add_mapping("LOCAL_1", "BROKER_1");

  auto local = mapper.get_local_id("BROKER_1");
  ASSERT_TRUE(local.has_value());
  EXPECT_EQ(*local, "LOCAL_1");
}

TEST(OrderIdMapper, UnknownLocalIDReturnsNullopt) {
  OrderIdMapper mapper;
  EXPECT_FALSE(mapper.get_broker_id("NO_SUCH_ID").has_value());
}

TEST(OrderIdMapper, UnknownBrokerIDReturnsNullopt) {
  OrderIdMapper mapper;
  EXPECT_FALSE(mapper.get_local_id("NO_SUCH_BROKER").has_value());
}

TEST(OrderIdMapper, RemoveMappingMakesBothLookupsFail) {
  OrderIdMapper mapper;
  mapper.add_mapping("LOCAL_2", "BROKER_2");
  mapper.remove_mapping("LOCAL_2");

  EXPECT_FALSE(mapper.get_broker_id("LOCAL_2").has_value());
  EXPECT_FALSE(mapper.get_local_id("BROKER_2").has_value());
}

TEST(OrderIdMapper, RemoveNonexistentMappingIsNoop) {
  OrderIdMapper mapper;
  EXPECT_NO_THROW(mapper.remove_mapping("GHOST_ID"));
}

TEST(OrderIdMapper, OverwritingMappingUpdatesLookup) {
  OrderIdMapper mapper;
  mapper.add_mapping("LOCAL_3", "BROKER_OLD");
  mapper.add_mapping("LOCAL_3", "BROKER_NEW");

  auto broker = mapper.get_broker_id("LOCAL_3");
  ASSERT_TRUE(broker.has_value());
  EXPECT_EQ(*broker, "BROKER_NEW");
}

TEST(OrderIdMapper, MultipleIndependentMappings) {
  OrderIdMapper mapper;
  mapper.add_mapping("A", "X");
  mapper.add_mapping("B", "Y");
  mapper.add_mapping("C", "Z");

  EXPECT_EQ(*mapper.get_broker_id("A"), "X");
  EXPECT_EQ(*mapper.get_broker_id("B"), "Y");
  EXPECT_EQ(*mapper.get_broker_id("C"), "Z");
  EXPECT_EQ(*mapper.get_local_id("X"), "A");
  EXPECT_EQ(*mapper.get_local_id("Y"), "B");
  EXPECT_EQ(*mapper.get_local_id("Z"), "C");
}

} // namespace quarcc
