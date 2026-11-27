# SPDX-License-Identifier: GPL-3.0-or-later
# zstd 1.5.7 (BSD-3-Clause): the shader preload pack (src/gfx/nvn/nvn_shaders.cpp) and the save checks of the game modules
# (games/fefw/fefw_core.cpp). Built with or without the graphics shell. The library sources only (no CLI, no x86-64 assembly).
# The archive is cached in NXRT_DEPS_DIR like the graphics dependencies (cmake/gfx_deps.cmake).
if(TARGET nxrt_zstd)
    return()
endif()
include(FetchContent)
set(NXRT_DEPS_DIR "$ENV{LOCALAPPDATA}/nxrt/deps" CACHE PATH "Where the pinned third-party sources are downloaded to")
set(FETCHCONTENT_QUIET OFF)
set(CMAKE_POLICY_DEFAULT_CMP0135 NEW)   # extracted files take the time of extraction
FetchContent_Declare(zstd
    URL https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-1.5.7.tar.gz
    DOWNLOAD_DIR "${NXRT_DEPS_DIR}"
    URL_HASH SHA256=eb33e51f49a15e023950cd7825ca74a4a2b43db8354825ac24fc1b7ee09e6fa3
    SOURCE_SUBDIR none)
FetchContent_MakeAvailable(zstd)

file(GLOB NXRT_ZSTD_SOURCES "${zstd_SOURCE_DIR}/lib/common/*.c" "${zstd_SOURCE_DIR}/lib/compress/*.c" "${zstd_SOURCE_DIR}/lib/decompress/*.c")
add_library(nxrt_zstd STATIC ${NXRT_ZSTD_SOURCES})
target_include_directories(nxrt_zstd SYSTEM PUBLIC "${zstd_SOURCE_DIR}/lib")
target_compile_definitions(nxrt_zstd PRIVATE ZSTD_DISABLE_ASM)
set_target_properties(nxrt_zstd PROPERTIES C_STANDARD 11)
