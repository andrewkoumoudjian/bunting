cmake_minimum_required(VERSION 3.24)

# Build modules in dependency-friendly order
add_subdirectory(persistence)
add_subdirectory(grpc)
add_subdirectory(observability)
add_subdirectory(gateways)
add_subdirectory(core)

if(TRADING_BUILD_APP)
    add_executable(trading_engine_app main.cpp)
    target_link_libraries(trading_engine_app PRIVATE trading_engine)
    trading_apply_warnings(trading_engine_app)
    set_target_properties(trading_engine_app PROPERTIES OUTPUT_NAME trading_engine)
endif()
