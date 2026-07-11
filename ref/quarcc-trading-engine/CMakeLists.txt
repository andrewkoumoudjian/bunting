cmake_minimum_required(VERSION 3.24)

project(trading_system
    VERSION 0.1.0
    DESCRIPTION "Modular trading system (C++ + Protobuf contracts)"
    LANGUAGES CXX
)

# ---- Global language/build settings ----
set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE RelWithDebInfo CACHE STRING "Build type" FORCE)
endif()

# ---- Feature toggles ----
option(TRADING_BUILD_APP               "Build engine-cpp/src/main.cpp executable" ON)
option(TRADING_ENABLE_GRPC             "Generate gRPC code from .proto files" ON)
option(TRADING_ENABLE_FIX_GATEWAY      "Enable Alpaca FIX gateway (QuickFIX required)" ON)
option(TRADING_ENABLE_WS_GATEWAY       "Enable websocket market data gateway (websocketpp required)" ON)
option(TRADING_ENABLE_PROMETHEUS       "Enable Prometheus exporter (prometheus-cpp required)" ON)

option(TRADING_ENABLE_ALPACA_SDK       "Enable Alpaca SDK integration" ON)
set(TRADING_ALPACA_SOURCE_DIR "" CACHE PATH "Optional local path to alpaca-sdk-cpp checkout")

if(TRADING_ENABLE_ALPACA_SDK)
    include(FetchContent)

    # SDK options from its README
    set(ALPACA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
    set(ALPACA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
    set(ALPACA_ENABLE_SSL ON CACHE BOOL "" FORCE)

    if(TRADING_ALPACA_SOURCE_DIR)
        # Use your local checkout
        FetchContent_Declare(
            alpaca_sdk
            SOURCE_DIR ${TRADING_ALPACA_SOURCE_DIR}
        )
    else()
        # Use GitHub
        FetchContent_Declare(
            alpaca_sdk
            GIT_REPOSITORY https://github.com/fatrifatrifat/alpaca-sdk-cpp.git
            GIT_TAG master
        )
    endif()

    FetchContent_MakeAvailable(alpaca_sdk)
endif()

# ---- Testing ----
option(TRADING_BUILD_TESTS "Build unit tests (Google Test + Google Mock)" ON)

if(TRADING_BUILD_TESTS)
    include(FetchContent)
    FetchContent_Declare(
        googletest
        GIT_REPOSITORY https://github.com/google/googletest.git
        GIT_TAG        v1.15.2
    )
    # For Windows: prevent overriding the parent project's compiler/linker settings
    set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
    FetchContent_MakeAvailable(googletest)
endif()

add_subdirectory(contracts)
add_subdirectory(engine-cpp)
