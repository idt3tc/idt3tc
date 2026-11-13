cmake_minimum_required(VERSION 3.19)

project(idtech3-toolchain
    VERSION 0.1.0
    DESCRIPTION "Tools to provision idTech 3 maps, mods, texture packs and pk3s"
    LANGUAGES CXX)

# The toolchain itself is strictly C++11 so it builds on old compilers that
# are still common in idTech 3 modding environments.
set(CMAKE_CXX_STANDARD 11)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

option(IDT3_BUILD_TESTS "Build the idt3tc unit and CLI tests" ON)
option(IDT3_WARNINGS_AS_ERRORS "Treat compiler warnings as errors for idt3tc sources" OFF)

# Default to an optimized build for single-config generators; image
# processing and block compression are painfully slow without optimization.
get_property(_idt3_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
if(NOT _idt3_multi_config AND NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

# Keep a*b+c as two rounded operations. GCC and clang otherwise fuse them
# into FMA instructions where the target has them (always on arm64, never on
# baseline x86_64), which changes the rounding and makes the same input
# produce different pixels and blocks depending on the platform. Applied
# before the dependencies so Compressonator is covered too.
if(CMAKE_CXX_COMPILER_ID MATCHES "Clang|GNU")
    add_compile_options(-ffp-contract=off)
endif()

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/cmake")
include(Dependencies)

find_package(Threads REQUIRED)

configure_file(src/common/version.h.in
    "${CMAKE_CURRENT_BINARY_DIR}/generated/common/version.h" @ONLY)

# All functionality lives in a static library so the tests can link against
# exactly the same code as the command line tool.
add_library(idt3_core STATIC
    src/common/fs.cpp
    src/common/parallel.cpp
    src/common/util.cpp
    src/img/bcn.cpp
    src/img/cmd_img.cpp
    src/img/color_ops.cpp
    src/img/context.cpp
    src/img/geometry_ops.cpp
    src/img/image.cpp
    src/img/io.cpp
    src/img/io_dds.cpp
    src/img/io_ops.cpp
    src/img/io_tga.cpp
    src/img/mipmap.cpp
    src/img/op.cpp
    src/img/options.cpp
    src/img/resample.cpp
    src/img/sampling.cpp
    src/img/spec.cpp
    src/img/stb_impl.cpp
    src/img/transform.cpp
)
target_include_directories(idt3_core PUBLIC
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${CMAKE_CURRENT_BINARY_DIR}/generated")
target_link_libraries(idt3_core
    PUBLIC Threads::Threads
    PRIVATE idt3::cmp_core idt3::stb)

if(MSVC)
    target_compile_options(idt3_core PRIVATE /W4 /utf-8)
    target_compile_definitions(idt3_core PRIVATE _CRT_SECURE_NO_WARNINGS NOMINMAX)
    if(IDT3_WARNINGS_AS_ERRORS)
        target_compile_options(idt3_core PRIVATE /WX)
    endif()
else()
    target_compile_options(idt3_core PRIVATE -Wall -Wextra -Wshadow)
    if(IDT3_WARNINGS_AS_ERRORS)
        target_compile_options(idt3_core PRIVATE -Werror)
    endif()
endif()

add_executable(idt3tc src/main.cpp)
target_link_libraries(idt3tc PRIVATE idt3_core)

include(GNUInstallDirs)
install(TARGETS idt3tc RUNTIME DESTINATION "${CMAKE_INSTALL_BINDIR}")

if(IDT3_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()
