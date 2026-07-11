cmake_minimum_required(VERSION 3.24)

find_package(Threads REQUIRED)
find_package(SQLite3 REQUIRED)
find_package(yaml-cpp REQUIRED)

# Shared interface target for include paths + common dependency propagation
add_library(trading_interfaces INTERFACE)
add_library(trading::interfaces ALIAS trading_interfaces)

target_compile_features(trading_interfaces INTERFACE cxx_std_23)

target_include_directories(trading_interfaces
    INTERFACE
        $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
        $<INSTALL_INTERFACE:include>
)

file(GLOB_RECURSE TRADING_PUBLIC_HEADERS CONFIGURE_DEPENDS
    ${CMAKE_CURRENT_SOURCE_DIR}/include/trading/*.h
    ${CMAKE_CURRENT_SOURCE_DIR}/include/trading/*.hpp
)

target_sources(trading_interfaces INTERFACE
    FILE_SET HEADERS
    BASE_DIRS ${CMAKE_CURRENT_SOURCE_DIR}/include
    FILES ${TRADING_PUBLIC_HEADERS}
)

# Contract headers (generated protobuf) are propagated through this link
target_link_libraries(trading_interfaces INTERFACE trading_contracts)

# Optional Alpaca SDK propagation (provided by root FetchContent)
if(TRADING_ENABLE_ALPACA_SDK)
    if(NOT TARGET alpaca::alpaca_sdk)
        message(FATAL_ERROR
            "TRADING_ENABLE_ALPACA_SDK=ON but alpaca::alpaca_sdk target not found. "
            "Make sure root CMake runs FetchContent_MakeAvailable(alpaca_sdk) before add_subdirectory(engine-cpp)."
        )
    endif()

    target_link_libraries(trading_interfaces INTERFACE alpaca::alpaca_sdk)
    target_compile_definitions(trading_interfaces INTERFACE TRADING_ENABLE_ALPACA_SDK=1)
endif()

# Helper for consistent warnings
function(trading_apply_warnings target_name)
    if(MSVC)
        target_compile_options(${target_name} PRIVATE /W4 /permissive-)
    else()
        target_compile_options(${target_name} PRIVATE -Wall -Wextra -Wpedantic)
    endif()
endfunction()

# Default capability flags (updated by module CMake files)
set(TRADING_WITH_FIX 0)
set(TRADING_WITH_WEBSOCKET 0)
set(TRADING_WITH_PROMETHEUS 0)

add_subdirectory(src)

target_link_libraries(trading_interfaces INTERFACE yaml-cpp)

if(TRADING_BUILD_TESTS)
    add_subdirectory(tests)
endif()
