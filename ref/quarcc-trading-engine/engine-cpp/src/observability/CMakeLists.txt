cmake_minimum_required(VERSION 3.24)

if(TRADING_ENABLE_PROMETHEUS)
    find_package(prometheus-cpp CONFIG QUIET)
endif()

if(TRADING_ENABLE_PROMETHEUS AND TARGET prometheus-cpp::core)
    add_library(trading_observability STATIC
        prometheus_exporter.cpp
    )

    target_link_libraries(trading_observability
        PUBLIC
            trading_interfaces
        PRIVATE
            prometheus-cpp::core
    )

    if(TARGET prometheus-cpp::pull)
        target_link_libraries(trading_observability PRIVATE prometheus-cpp::pull)
    endif()

    trading_apply_warnings(trading_observability)

    set(TRADING_WITH_PROMETHEUS 1 PARENT_SCOPE)
else()
    add_library(trading_observability INTERFACE)
    target_link_libraries(trading_observability INTERFACE trading_interfaces)

    if(TRADING_ENABLE_PROMETHEUS)
        message(WARNING "TRADING_ENABLE_PROMETHEUS=ON but prometheus-cpp not found. Building without Prometheus exporter.")
    else()
        message(STATUS "Prometheus exporter disabled.")
    endif()

    set(TRADING_WITH_PROMETHEUS 0 PARENT_SCOPE)
endif()

add_library(trading::observability ALIAS trading_observability)
