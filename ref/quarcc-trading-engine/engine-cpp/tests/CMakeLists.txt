set(TRADING_TEST_SOURCES
    unit/test_position_keeper.cpp
    unit/test_order_id_mapper.cpp
    unit/test_order_manager.cpp
    unit/test_sqlite_journal.cpp
    unit/test_sqlite_order_store.cpp
)

add_executable(trading_tests ${TRADING_TEST_SOURCES})

# Tests directory for mocks/ and helpers/ headers
target_include_directories(trading_tests PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(trading_tests PRIVATE
    trading_core
    trading_persistence
    trading_interfaces
    GTest::gtest_main
    GTest::gmock
)

trading_apply_warnings(trading_tests)

include(GoogleTest)
gtest_discover_tests(trading_tests)
