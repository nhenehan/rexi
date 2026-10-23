# C smoke test for the rexi C API. Driven by crates/rexi-capi/tests/c_smoke.rs:
#   cmake -S . -B build -DREXI_INCLUDE_DIR=<crate>/include -DREXI_LIB=<target>/librexi.a
#   cmake --build build && ctest --test-dir build --output-on-failure
cmake_minimum_required(VERSION 3.16)
project(rexi_smoke C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(REXI_INCLUDE_DIR "" CACHE PATH "Directory containing rexi.h")
set(REXI_LIB "" CACHE FILEPATH "Path to librexi.a")
set(REXI_FIXTURES "" CACHE PATH "crates/rexi-tests/fixtures/interop")
option(REXI_SANITIZE "Build the C side with AddressSanitizer and UBSan" OFF)

if(NOT REXI_INCLUDE_DIR OR NOT REXI_LIB)
  message(FATAL_ERROR "set REXI_INCLUDE_DIR and REXI_LIB")
endif()

add_executable(rexi_smoke smoke.c)
target_include_directories(rexi_smoke PRIVATE ${REXI_INCLUDE_DIR})
target_compile_options(rexi_smoke PRIVATE -Wall -Wextra -Werror)
find_package(Threads REQUIRED)
target_link_libraries(rexi_smoke PRIVATE ${REXI_LIB} Threads::Threads ${CMAKE_DL_LIBS} m)
if(REXI_SANITIZE)
  target_compile_options(rexi_smoke PRIVATE -fsanitize=address,undefined -fno-omit-frame-pointer)
  target_link_options(rexi_smoke PRIVATE -fsanitize=address,undefined)
endif()

enable_testing()
add_test(NAME rexi_smoke COMMAND rexi_smoke ${REXI_FIXTURES})
