cmake_minimum_required(VERSION 3.24)

add_library(trading_persistence STATIC
    sqlite_order_store.cpp
    sqlite_journal.cpp
)

add_library(trading::persistence ALIAS trading_persistence)

target_link_libraries(trading_persistence
    PUBLIC
        trading_interfaces
        SQLite3::SQLite3
        Threads::Threads
)

trading_apply_warnings(trading_persistence)
