# Third-party dependencies, fetched as pinned and hash-verified source
# archives. To build offline, point FetchContent at local checkouts, e.g.
#   -DFETCHCONTENT_SOURCE_DIR_COMPRESSONATOR=/path/to/compressonator
#   -DFETCHCONTENT_SOURCE_DIR_STB=/path/to/stb

include(FetchContent)

# Use the extraction time as file timestamps so that re-extracting a changed
# archive reliably triggers rebuilds.
if(POLICY CMP0135)
    cmake_policy(SET CMP0135 NEW)
endif()

set(IDT3_COMPRESSONATOR_URL
    "https://github.com/GPUOpen-Tools/compressonator/archive/refs/tags/V4.6.0.tar.gz"
    CACHE STRING "Compressonator source archive")
set(IDT3_COMPRESSONATOR_SHA256
    "4e64d84336689763a107feff76275d59080a329d14242c55e2273f7319530743"
    CACHE STRING "SHA256 of the Compressonator source archive")

set(IDT3_STB_URL
    "https://github.com/nothings/stb/archive/9600522de53973336fa033a289858a4354cbb04c.tar.gz"
    CACHE STRING "stb source archive")
set(IDT3_STB_SHA256
    "8546b1281cf61e4b5229e9eb05055d765f1b076f6ba03aab377df09a20fb613b"
    CACHE STRING "SHA256 of the stb source archive")

# Archives are downloaded and extracted once into a cache shared by every
# build directory, so building for all target platforms does not fetch them
# once per platform. The extraction directory is keyed by the archive hash:
# bumping a pin extracts afresh instead of reusing the old tree.
set(IDT3_DEPS_CACHE "${PROJECT_SOURCE_DIR}/.cache/deps" CACHE PATH
    "Shared download and extraction cache for fetched dependencies")

# Declares one dependency. `probe` is a file that only exists once the archive
# has been extracted completely; when it is there, FetchContent is pointed
# straight at the cached tree and neither downloads nor extracts anything.
#
# SOURCE_SUBDIR names a directory without a CMakeLists.txt so that
# FetchContent_MakeAvailable() only downloads and extracts. Compressonator's
# own build pulls in its whole SDK and application stack; only the CMP_Core
# block codecs are needed, and those get a target of their own below.
function(idt3_declare_archive name url sha256 probe)
    string(TOUPPER "${name}" _upper)
    string(SUBSTRING "${sha256}" 0 16 _short)
    set(_src "${IDT3_DEPS_CACHE}/${name}-${_short}")
    if(NOT FETCHCONTENT_SOURCE_DIR_${_upper} AND EXISTS "${_src}/${probe}")
        set(FETCHCONTENT_SOURCE_DIR_${_upper} "${_src}" PARENT_SCOPE)
    endif()
    FetchContent_Declare(${name}
        URL "${url}"
        URL_HASH "SHA256=${sha256}"
        DOWNLOAD_DIR "${IDT3_DEPS_CACHE}/download"
        SOURCE_DIR "${_src}"
        SOURCE_SUBDIR "idt3-no-cmake")
endfunction()

idt3_declare_archive(compressonator "${IDT3_COMPRESSONATOR_URL}" "${IDT3_COMPRESSONATOR_SHA256}"
    "cmp_core/source/cmp_core.cpp")
idt3_declare_archive(stb "${IDT3_STB_URL}" "${IDT3_STB_SHA256}" "stb_image_write.h")
FetchContent_MakeAvailable(compressonator stb)

# --- Compressonator CMP_Core (BC1-BC7 CPU block encoders/decoders) ---------
# Upstream's CPUID based SIMD dispatch is replaced (see cmp_cpu_extensions.cpp)
# so that every platform runs the same scalar code; the SIMD variants of the
# BC1 endpoint search are therefore never compiled, only stubbed for the
# linker.
# CMP_Core is compiled from a copy of the three source directories it needs,
# in the build directory, with the fixes below applied. Copying the whole
# directories rather than single files matters because the sources include
# their headers with quotes, which always finds the sibling file first. The
# shared extraction cache (or a checkout given through
# FETCHCONTENT_SOURCE_DIR_COMPRESSONATOR) is never modified.
set(_idt3_cmp_upstream "${compressonator_SOURCE_DIR}")
set(_idt3_cmp "${CMAKE_CURRENT_BINARY_DIR}/compressonator")

# Each patch is FILE OLD NEW: an exact text substitution in FILE (relative to
# the Compressonator root). OLD must occur, so a version bump that moves the
# code fails loudly instead of silently dropping the fix.
set(_idt3_cmp_patches
    # pack_index() shifts a 32-bit value by up to 60 bits. GCC on x86 happens
    # to produce what was meant (the hardware masks the count to 5 bits);
    # clang treats the path as unreachable and the encoder traps on the first
    # BC7 mode 4/5 block. k / 8 already selects the word, so masking is what
    # was intended.
    "cmp_core/shaders/bc7_encode_kernel.cpp"
    "(CGV_UINT32)(src_index[k] & 0x0F) << shift;"
    "(CGV_UINT32)(src_index[k] & 0x0F) << (shift & 31);"
    # The BC3/BC4/BC5 alpha index packing shifts signed 3-bit indices into
    # bit 31 and beyond, which overflows int; the target words are unsigned.
    "cmp_core/shaders/bcn_common_kernel.h"
    "cmpBlock.x |= (pcIndices[i] << (16 + (i * 3)));"
    "cmpBlock.x |= ((CGU_UINT32)pcIndices[i] << (16 + (i * 3)));"
    "cmp_core/shaders/bcn_common_kernel.h"
    "cmpBlock.x |= (pcIndices[5] << 31);"
    "cmpBlock.x |= ((CGU_UINT32)pcIndices[5] << 31);"
    "cmp_core/shaders/bcn_common_kernel.h"
    "cmpBlock.y |= (pcIndices[i] << (i * 3 - 16));"
    "cmpBlock.y |= ((CGU_UINT32)pcIndices[i] << (i * 3 - 16));"
)

# Writes `content` to `path` only when it differs, so reconfiguring does not
# touch timestamps and trigger a full rebuild.
function(_idt3_write_if_changed path content)
    if(EXISTS "${path}")
        file(READ "${path}" _old)
        if(_old STREQUAL content)
            return()
        endif()
    endif()
    file(WRITE "${path}" "${content}")
endfunction()

function(_idt3_prepare_cmp_tree)
    set(_patches ${_idt3_cmp_patches})
    foreach(_dir cmp_core/shaders cmp_core/source applications/_libs/cmp_math)
        file(GLOB _files RELATIVE "${_idt3_cmp_upstream}" "${_idt3_cmp_upstream}/${_dir}/*")
        foreach(_file IN LISTS _files)
            if(IS_DIRECTORY "${_idt3_cmp_upstream}/${_file}")
                continue()
            endif()
            file(READ "${_idt3_cmp_upstream}/${_file}" _content)
            list(LENGTH _patches _count)
            set(_i 0)
            while(_i LESS _count)
                math(EXPR _o "${_i} + 1")
                math(EXPR _n "${_i} + 2")
                list(GET _patches ${_i} _target)
                if(_target STREQUAL _file)
                    list(GET _patches ${_o} _old)
                    list(GET _patches ${_n} _new)
                    string(FIND "${_content}" "${_old}" _at)
                    if(_at EQUAL -1)
                        message(FATAL_ERROR "Compressonator patch not applicable to ${_file}: '${_old}'")
                    endif()
                    string(REPLACE "${_old}" "${_new}" _content "${_content}")
                endif()
                math(EXPR _i "${_i} + 3")
            endwhile()
            _idt3_write_if_changed("${_idt3_cmp}/${_file}" "${_content}")
        endforeach()
    endforeach()
endfunction()
_idt3_prepare_cmp_tree()

add_library(idt3_cmp_core STATIC
    "${_idt3_cmp}/cmp_core/shaders/bc1_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc2_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc3_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc4_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc5_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc6_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc7_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/source/cmp_core.cpp"
    "${_idt3_cmp}/applications/_libs/cmp_math/cmp_math_common.cpp"
    "${CMAKE_CURRENT_LIST_DIR}/cmp_cpu_extensions.cpp"
    "${CMAKE_CURRENT_LIST_DIR}/cmp_simd_stubs.cpp")
target_include_directories(idt3_cmp_core
    SYSTEM PUBLIC "${_idt3_cmp}/cmp_core/source"
    PRIVATE
        "${_idt3_cmp}/cmp_core/shaders"
        "${_idt3_cmp}/applications/_libs/cmp_math")
# Same defines as upstream's CMP_Core target.
if(UNIX)
    target_compile_definitions(idt3_cmp_core PRIVATE _LINUX ASPM_GPU)
endif()

# On Windows x86, cmp_floor() uses the SSE4.1 _mm_floor_ss intrinsic, which
# MSVC accepts without any flag but clang only for an SSE4.1 target. Rather
# than raising the CPU baseline of the whole library, an SSE2 equivalent is
# put in place before the code is seen.
if(WIN32 AND NOT MSVC AND CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64|amd64|i[3-6]86|x86)$")
    target_compile_options(idt3_cmp_core PRIVATE
        "SHELL:-include \"${CMAKE_CURRENT_LIST_DIR}/cmp_win_x86_compat.h\"")
endif()

if(MSVC)
    target_compile_options(idt3_cmp_core PRIVATE /W0)
else()
    target_compile_options(idt3_cmp_core PRIVATE -w)
endif()
set_target_properties(idt3_cmp_core PROPERTIES POSITION_INDEPENDENT_CODE ON)
add_library(idt3::cmp_core ALIAS idt3_cmp_core)

# --- stb (image decoding / PNG, JPG, BMP encoding) -------------------------
# Header-only; the implementation is compiled once in src/img/stb_impl.cpp.
add_library(idt3_stb INTERFACE)
target_include_directories(idt3_stb SYSTEM INTERFACE "${stb_SOURCE_DIR}")
add_library(idt3::stb ALIAS idt3_stb)
