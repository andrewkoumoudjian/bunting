cmake_minimum_required(VERSION 3.24)

add_library(trading_grpc STATIC
  grpc_server.cpp
)

add_library(trading::grpc ALIAS trading_grpc)

target_link_libraries(trading_grpc
    PUBLIC
        trading_interfaces
        SQLite3::SQLite3
        Threads::Threads
)

trading_apply_warnings(trading_grpc)
