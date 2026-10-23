# The rexi C API example. Build the library first, from the rexi folder:
#   cargo build --release -p rexi-capi
# then, from examples/basics:
#   cmake -S c -B c/build && cmake --build c/build && c/build/order data/order.xsd
cmake_minimum_required(VERSION 3.16)
project(rexi_c_example C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(REXI_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../.." CACHE PATH "The rexi folder")
set(REXI_INCLUDE_DIR "${REXI_ROOT}/crates/rexi-capi/include" CACHE PATH "Folder with rexi.h")
set(REXI_LIB "${REXI_ROOT}/target/release/librexi.a" CACHE FILEPATH "Path to librexi.a")

add_executable(order order.c)
target_include_directories(order PRIVATE ${REXI_INCLUDE_DIR})
target_compile_options(order PRIVATE -Wall -Wextra -Werror)
# The static library is Rust code, which needs threads, dl and libm.
find_package(Threads REQUIRED)
target_link_libraries(order PRIVATE ${REXI_LIB} Threads::Threads ${CMAKE_DL_LIBS} m)
