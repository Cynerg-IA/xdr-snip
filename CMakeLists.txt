cmake_minimum_required(VERSION 3.20)
project(xdr_snip_cpp LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)   # no GCC-only extensions; keep portable to MSVC

if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

# ---------------------------------------------------------------------------
# toml++ (header-only, MIT). Phase 1 uses CMake FetchContent to pull it from
# GitHub at configure time -- this was verified to work from the
# xdrsnip-jxl-size:v2 toolchain container (network egress confirmed reachable
# to raw.githubusercontent.com/github.com during Phase 1 development,
# 2026-08-19). If a future CI/build environment has no network egress, fall
# back to vendoring the single-header toml.hpp under
# third_party/tomlplusplus/ and swap the FetchContent block below for an
# add_library(tomlplusplus INTERFACE) pointing at the vendored header.
# Do NOT hand-roll a TOML parser.
# ---------------------------------------------------------------------------
include(FetchContent)
FetchContent_Declare(
  tomlplusplus
  GIT_REPOSITORY https://github.com/marzer/tomlplusplus.git
  GIT_TAG        v3.4.0
)
FetchContent_MakeAvailable(tomlplusplus)

add_library(snip_types STATIC
  src/types.cpp
  src/config.cpp
)
target_include_directories(snip_types PUBLIC src)
target_link_libraries(snip_types PUBLIC tomlplusplus::tomlplusplus)

enable_testing()

add_executable(snip_types_tests
  tests/test_main.cpp
)
target_link_libraries(snip_types_tests PRIVATE snip_types)

add_test(NAME snip_types_tests COMMAND snip_types_tests)
