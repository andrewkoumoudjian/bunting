cmake_minimum_required(VERSION 3.24)

# Core business logic (excluding top-level orchestrator .cpp)
add_library(trading_core STATIC
    order_manager.cpp
    position_keeper.cpp
    risk_manager.cpp
    poll_scheduler.cpp
    feed_registry.cpp
)

add_library(trading::core ALIAS trading_core)

target_link_libraries(trading_core
    PUBLIC
        trading_interfaces
    PRIVATE
        Threads::Threads
)

trading_apply_warnings(trading_core)

# Orchestrator target
add_library(trading_engine STATIC
    trading_engine.cpp
)

add_library(trading::engine ALIAS trading_engine)

target_link_libraries(trading_engine
    PUBLIC
        trading_core
    PRIVATE
        trading_gateways
        trading_grpc
        trading_persistence
        trading_observability
        Threads::Threads
)

# Propagate feature flags so code can #if on optional modules
target_compile_definitions(trading_engine
    PUBLIC
        TRADING_WITH_FIX=${TRADING_WITH_FIX}
        TRADING_WITH_WEBSOCKET=${TRADING_WITH_WEBSOCKET}
        TRADING_WITH_PROMETHEUS=${TRADING_WITH_PROMETHEUS}
)

trading_apply_warnings(trading_engine)
