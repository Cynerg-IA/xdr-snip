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

# ---------------------------------------------------------------------------
# libjxl (Phase 2, issue #7/#8). Built from the pre-existing source checkout
# at /work/libjxl (baked into the xdrsnip-jxl-size:v2 toolchain image by the
# Phase 0 spike) via add_subdirectory, NOT FetchContent -- no network fetch
# needed since the checkout is already local to the build container.
#
# LIBJXL_SOURCE_DIR is settable so this also works in a different toolchain
# image or a future CI runner that vendors libjxl at a different path;
# defaults to the known toolchain-image location.
#
# Trim profile is the issue #7 Section 0 "§0 trim profile" (encoder-only,
# reduced Highway SIMD targets, skcms over lcms2, no JPEG transcode, no
# boxes, no tools/benchmark/examples/docs/JNI/sjpeg/OpenEXR-in-libjxl). We
# ADDITIONALLY enable the decoder (JPEGXL_ENABLE_TOOLS stays off, but the
# jxl_dec target is still produced as part of the default lib build) purely
# so this repo's own Phase 2 tests can round-trip-verify lossless encoding
# and assert on decoded JxlBasicInfo -- the shipped xdr-snip.exe only ever
# links jxl_enc (see target_link_libraries(snip_encode_jxl ...) below),
# jxl_dec is test-only and never linked into the production static lib.
#
# KNOWN TRAP (Phase 0, issue #7): the jxl_threads target FAILS to compile on
# GCC/mingw (missing <mutex>/<condition_variable> includes in
# thread_parallel_runner_internal.h / resizable_parallel_runner.cc). We do
# NOT link jxl_threads anywhere in this repo -- encode_jxl.cpp deliberately
# never calls JxlEncoderSetParallelRunner, so it is never needed. CMake
# still configures the jxl_threads target as part of libjxl's own
# lib/CMakeLists.txt; that's fine as long as nothing tries to link it, which
# nothing here does.
# ---------------------------------------------------------------------------
if(NOT DEFINED LIBJXL_SOURCE_DIR)
  set(LIBJXL_SOURCE_DIR "/work/libjxl" CACHE PATH "Path to a libjxl source checkout")
endif()

if(EXISTS "${LIBJXL_SOURCE_DIR}/CMakeLists.txt")
  set(JPEGXL_ENABLE_TOOLS OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_BENCHMARK OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_EXAMPLES OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_PLUGINS OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_MANPAGES OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_DOXYGEN OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_JNI OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_SJPEG OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_OPENEXR OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_TRANSCODE_JPEG OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_BOXES OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_SKCMS ON CACHE BOOL "" FORCE)
  set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
  set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)

  # Highway SIMD targets: keep only SSE4 + AVX2 (+ scalar/EMU128 fallback
  # libjxl always keeps). Disabling the rest is the DOMINANT exe-size lever
  # per issue #7's measured Phase 0 numbers.
  set(JPEGXL_ENABLE_HWY_AVX3 OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_HWY_AVX3_DL OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_HWY_AVX3_SPR OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_HWY_AVX3_ZEN4 OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_HWY_SSSE3 OFF CACHE BOOL "" FORCE)
  set(JPEGXL_ENABLE_HWY_SSE2 OFF CACHE BOOL "" FORCE)

  add_subdirectory(${LIBJXL_SOURCE_DIR} ${CMAKE_BINARY_DIR}/libjxl-build EXCLUDE_FROM_ALL)

  # libjxl's jxl_enc/jxl_dec static-lib targets do NOT propagate public
  # include dirs (only the internal *-obj targets do, in jxl.cmake) -- so
  # both the checked-out headers (lib/include) and the build-tree-generated
  # headers (e.g. jxl/version.h, expanded from version.h.in at configure
  # time) must be added explicitly.
  set(JXL_PUBLIC_INCLUDE_DIRS
    "${LIBJXL_SOURCE_DIR}/lib/include"
    "${CMAKE_BINARY_DIR}/libjxl-build/lib/include"
  )

  add_library(snip_encode_jxl STATIC
    src/encode_jxl.cpp
  )
  target_include_directories(snip_encode_jxl PUBLIC src)
  target_include_directories(snip_encode_jxl PRIVATE ${JXL_PUBLIC_INCLUDE_DIRS})
  # NOTE on "encoder-only": libjxl's own lib/jxl.cmake (verified by reading
  # it directly, not guessed) does NOT produce a standalone encoder-only
  # static-lib TARGET -- JxlEncoder* lives in the jxl_enc-obj OBJECT
  # library, which is only ever bundled into the combined `jxl` target
  # (encoder+decoder) or the private jxl-internal target used by libjxl's
  # own tests. There is no public `jxl_enc` static archive to link against;
  # that name does not exist in upstream CMakeLists (confirmed by grepping
  # jxl.cmake -- only `jxl`, `jxl_dec`, `jxl-internal`, `jxl_dec-internal`,
  # and the two *-obj object libraries exist). So "link jxl_enc, drop
  # decoder paths" from issue #7 is only achievable by consuming
  # jxl_enc-obj's objects directly; we use the public `jxl` target instead
  # for simplicity and stability across libjxl versions, at the cost of
  # bundling decode support into the production static lib too. Flagged
  # explicitly in the Phase 2 report -- this is a real deviation from the
  # spec's literal "link jxl_enc" instruction, forced by libjxl's actual
  # target graph, not an oversight.
  #
  # PUBLIC, not PRIVATE: snip_encode_jxl is a static lib, and static libs do
  # not bundle their own dependencies' object code for downstream linkers --
  # anything that links snip_encode_jxl (e.g. snip_encode_jxl_tests below,
  # and eventually the real xdr-snip.exe in a later phase) must also see
  # jxl on its own link line, which PUBLIC propagates automatically.
  target_link_libraries(snip_encode_jxl PUBLIC snip_types jxl)

  add_executable(snip_encode_jxl_tests
    tests/test_encode_jxl.cpp
  )
  target_include_directories(snip_encode_jxl_tests PRIVATE ${JXL_PUBLIC_INCLUDE_DIRS})
  target_link_libraries(snip_encode_jxl_tests PRIVATE snip_encode_jxl jxl_dec)

  add_test(NAME snip_encode_jxl_tests COMMAND snip_encode_jxl_tests)
else()
  message(WARNING
    "LIBJXL_SOURCE_DIR (${LIBJXL_SOURCE_DIR}) has no CMakeLists.txt -- "
    "skipping the JXL encoder target (snip_encode_jxl) and its tests. "
    "Set -DLIBJXL_SOURCE_DIR=/path/to/libjxl to enable Phase 2.")
endif()
