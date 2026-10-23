# todo-c-client: the MQTT todo example's C client.
# Eclipse Paho MQTT C (sync client, MQTT 3.1.1) + rexi's C API.
#
#   ../scripts/fetch-third-party.sh                  (Paho C, pinned)
#   cargo build --release -p rexi-capi --lib         (in the rexi root: librexi.a)
#   cmake -S . -B build -DPAHO_DIR=<cache>/paho.mqtt.c && cmake --build build
#
# ../build.sh does all of this.
cmake_minimum_required(VERSION 3.16)
project(todo_c_client C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

get_filename_component(REXI_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
set(REXI_INCLUDE_DIR "${REXI_ROOT}/crates/rexi-capi/include" CACHE PATH "Directory containing rexi.h")
set(REXI_LIB "${REXI_ROOT}/target/release/librexi.a" CACHE FILEPATH "Path to librexi.a")
set(PAHO_DIR "" CACHE PATH "Eclipse Paho MQTT C checkout (scripts/fetch-third-party.sh)")
if(NOT EXISTS "${REXI_LIB}")
  message(FATAL_ERROR "${REXI_LIB} is missing: run `cargo build --release -p rexi-capi --lib` in ${REXI_ROOT}")
endif()
if(NOT PAHO_DIR OR NOT EXISTS "${PAHO_DIR}/src/MQTTClient.h")
  message(FATAL_ERROR "set PAHO_DIR to the paho.mqtt.c checkout made by ../scripts/fetch-third-party.sh")
endif()

# Paho C as part of this build: the static synchronous client only.
set(PAHO_BUILD_STATIC ON CACHE BOOL "" FORCE)
set(PAHO_BUILD_SHARED OFF CACHE BOOL "" FORCE)
set(PAHO_WITH_SSL OFF CACHE BOOL "" FORCE)
set(PAHO_ENABLE_TESTING OFF CACHE BOOL "" FORCE)
set(PAHO_ENABLE_CPACK OFF CACHE BOOL "" FORCE)
set(PAHO_BUILD_SAMPLES OFF CACHE BOOL "" FORCE)
add_subdirectory("${PAHO_DIR}" paho EXCLUDE_FROM_ALL)

add_executable(todo-c-client todo_client.c)
target_include_directories(todo-c-client PRIVATE ${REXI_INCLUDE_DIR} "${PAHO_DIR}/src" "${CMAKE_BINARY_DIR}/paho/src")
# The schema next to this directory is the default; --xsd picks another.
target_compile_definitions(todo-c-client PRIVATE TODO_XSD="${CMAKE_CURRENT_SOURCE_DIR}/../todo.xsd")
target_compile_options(todo-c-client PRIVATE -Wall -Wextra -Werror)
find_package(Threads REQUIRED)
target_link_libraries(todo-c-client PRIVATE paho-mqtt3c-static ${REXI_LIB} Threads::Threads ${CMAKE_DL_LIBS} m)
