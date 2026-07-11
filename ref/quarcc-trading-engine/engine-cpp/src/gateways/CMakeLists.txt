cmake_minimum_required(VERSION 3.24)

set(GATEWAY_SOURCES
    paper_trading_gateway.cpp
    simulated_feed.cpp
)

if(TRADING_ENABLE_ALPACA_SDK)
    list(APPEND GATEWAY_SOURCES alpaca_fix_gateway.cpp)
endif()

set(_HAS_FIX 0)
set(_HAS_WS 0)

# ---- Optional QuickFIX gateway ----
if(TRADING_ENABLE_FIX_GATEWAY)
    find_package(QuickFIX QUIET)
    if(QuickFIX_FOUND)
        list(APPEND GATEWAY_SOURCES alpaca_fix_gateway.cpp)
        set(_HAS_FIX 1)
    else()
        message(WARNING "TRADING_ENABLE_FIX_GATEWAY=ON but QuickFIX not found. Skipping alpaca_fix_gateway.cpp.")
    endif()
endif()

# ---- Optional websocket gateway ----
# if(TRADING_ENABLE_WS_GATEWAY)
#     find_package(websocketpp CONFIG QUIET)
#     if(websocketpp_FOUND)
#         find_package(OpenSSL REQUIRED)
#         list(APPEND GATEWAY_SOURCES websocket_md_gateway.cpp)
#         set(_HAS_WS 1)
#     else()
#         message(WARNING "TRADING_ENABLE_WS_GATEWAY=ON but websocketpp not found. Skipping websocket_md_gateway.cpp.")
#     endif()
# endif()

add_library(trading_gateways STATIC ${GATEWAY_SOURCES})
add_library(trading::gateways ALIAS trading_gateways)

target_link_libraries(trading_gateways
    PUBLIC
        trading_interfaces
        Threads::Threads
)

# Link QuickFIX (target name can vary by package manager)
if(_HAS_FIX)
    if(TARGET QuickFIX::QuickFIX)
        target_link_libraries(trading_gateways PRIVATE QuickFIX::QuickFIX)
    elseif(TARGET QuickFIX::quickfix)
        target_link_libraries(trading_gateways PRIVATE QuickFIX::quickfix)
    elseif(DEFINED QuickFIX_LIBRARIES)
        target_link_libraries(trading_gateways PRIVATE ${QuickFIX_LIBRARIES})
        if(DEFINED QuickFIX_INCLUDE_DIRS)
            target_include_directories(trading_gateways PRIVATE ${QuickFIX_INCLUDE_DIRS})
        endif()
    else()
        message(WARNING "QuickFIX found but no known CMake target exposed.")
    endif()
endif()

# Link websocket stack
if(_HAS_WS)
    if(TARGET websocketpp::websocketpp)
        target_link_libraries(trading_gateways PRIVATE websocketpp::websocketpp)
    endif()
    target_link_libraries(trading_gateways PRIVATE OpenSSL::SSL OpenSSL::Crypto)
endif()

target_compile_definitions(trading_gateways
    PUBLIC
        TRADING_WITH_FIX=${_HAS_FIX}
        TRADING_WITH_WEBSOCKET=${_HAS_WS}
)

trading_apply_warnings(trading_gateways)

# Export capability flags upward for core/app
set(TRADING_WITH_FIX ${_HAS_FIX} PARENT_SCOPE)
set(TRADING_WITH_WEBSOCKET ${_HAS_WS} PARENT_SCOPE)
