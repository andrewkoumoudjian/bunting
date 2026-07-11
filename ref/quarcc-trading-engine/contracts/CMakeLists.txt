cmake_minimum_required(VERSION 3.24)

# Keep this if you want protobuf_generate_cpp fallback in CONFIG mode.
set(protobuf_MODULE_COMPATIBLE TRUE)

find_package(Protobuf CONFIG REQUIRED)

set(TRADING_PROTO_FILES
    ${CMAKE_CURRENT_SOURCE_DIR}/common.proto
    ${CMAKE_CURRENT_SOURCE_DIR}/strategy_signal.proto
    ${CMAKE_CURRENT_SOURCE_DIR}/order.proto
    ${CMAKE_CURRENT_SOURCE_DIR}/execution.proto
    ${CMAKE_CURRENT_SOURCE_DIR}/market_data.proto
    ${CMAKE_CURRENT_SOURCE_DIR}/execution_service.proto
)

set(TRADING_PROTO_GEN_DIR ${CMAKE_CURRENT_BINARY_DIR}/generated)
file(MAKE_DIRECTORY ${TRADING_PROTO_GEN_DIR})

add_library(trading_contracts STATIC)
add_library(trading::contracts ALIAS trading_contracts)

target_link_libraries(trading_contracts PUBLIC protobuf::libprotobuf)

# Generate C++ protobuf sources
if(COMMAND protobuf_generate)
    protobuf_generate(
        TARGET         trading_contracts
        LANGUAGE       cpp
        PROTOS         ${TRADING_PROTO_FILES}
        IMPORT_DIRS    ${CMAKE_CURRENT_SOURCE_DIR}
        PROTOC_OUT_DIR ${TRADING_PROTO_GEN_DIR}
    )
elseif(COMMAND protobuf_generate_cpp)
    # Legacy fallback
    protobuf_generate_cpp(TRADING_PROTO_SRCS TRADING_PROTO_HDRS ${TRADING_PROTO_FILES})
    target_sources(trading_contracts PRIVATE ${TRADING_PROTO_SRCS} ${TRADING_PROTO_HDRS})
    set(TRADING_PROTO_GEN_DIR ${CMAKE_CURRENT_BINARY_DIR})
else()
    message(FATAL_ERROR "No protobuf generation function is available (protobuf_generate/protobuf_generate_cpp).")
endif()

# Optional gRPC generation
if(TRADING_ENABLE_GRPC)
    find_package(gRPC CONFIG REQUIRED)

    if(TARGET gRPC::grpc_cpp_plugin)
        set(_GRPC_CPP_PLUGIN_EXECUTABLE $<TARGET_FILE:gRPC::grpc_cpp_plugin>)
    else()
        find_program(_GRPC_CPP_PLUGIN_EXECUTABLE grpc_cpp_plugin)
    endif()

    if(_GRPC_CPP_PLUGIN_EXECUTABLE)
        protobuf_generate(
            TARGET               trading_contracts
            LANGUAGE             grpc
            GENERATE_EXTENSIONS  .grpc.pb.h .grpc.pb.cc
            PLUGIN               "protoc-gen-grpc=${_GRPC_CPP_PLUGIN_EXECUTABLE}"
            PROTOS               ${TRADING_PROTO_FILES}
            IMPORT_DIRS          ${CMAKE_CURRENT_SOURCE_DIR}
            PROTOC_OUT_DIR       ${TRADING_PROTO_GEN_DIR}
        )
        target_link_libraries(trading_contracts PUBLIC gRPC::grpc++ gRPC::grpc)
    else()
        message(FATAL_ERROR "gRPC requested but grpc_cpp_plugin not found.")
    endif()
endif()

target_include_directories(trading_contracts
    PUBLIC
        $<BUILD_INTERFACE:${TRADING_PROTO_GEN_DIR}>
        $<INSTALL_INTERFACE:include/trading/contracts>
)
