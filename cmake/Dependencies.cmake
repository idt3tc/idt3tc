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

# SOURCE_SUBDIR points at a directory without a CMakeLists.txt so that
# FetchContent_MakeAvailable() only downloads and extracts. Compressonator's
# own build pulls in its whole SDK and application stack; only the CMP_Core
# block codecs are needed, and those get a target of their own below.
FetchContent_Declare(compressonator
    URL "${IDT3_COMPRESSONATOR_URL}"
    URL_HASH "SHA256=${IDT3_COMPRESSONATOR_SHA256}"
    SOURCE_SUBDIR "idt3-no-cmake")
FetchContent_Declare(stb
    URL "${IDT3_STB_URL}"
    URL_HASH "SHA256=${IDT3_STB_SHA256}"
    SOURCE_SUBDIR "idt3-no-cmake")
FetchContent_MakeAvailable(compressonator stb)

# --- Compressonator CMP_Core (BC1-BC7 CPU block encoders/decoders) ---------
set(_idt3_cmp "${compressonator_SOURCE_DIR}")
add_library(idt3_cmp_core STATIC
    "${_idt3_cmp}/cmp_core/shaders/bc1_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc2_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc3_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc4_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc5_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc6_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/shaders/bc7_encode_kernel.cpp"
    "${_idt3_cmp}/cmp_core/source/cmp_core.cpp"
    "${_idt3_cmp}/applications/_libs/cmp_math/cpu_extensions.cpp"
    "${_idt3_cmp}/applications/_libs/cmp_math/cmp_math_common.cpp")
target_include_directories(idt3_cmp_core
    SYSTEM PUBLIC "${_idt3_cmp}/cmp_core/source"
    PRIVATE
        "${_idt3_cmp}/cmp_core/shaders"
        "${_idt3_cmp}/applications/_libs/cmp_math")
# Same defines as upstream's CMP_Core target.
if(UNIX)
    target_compile_definitions(idt3_cmp_core PRIVATE _LINUX ASPM_GPU)
endif()

# The BC1 encoder dispatches at runtime to SSE4.2/AVX2/AVX-512 variants of
# its endpoint search (on Linux upstream's CPU detection reports no
# extensions, so the scalar path is used there). On x86 the variants are
# compiled with just the ISA flags they need (upstream uses -march=knl
# for AVX-512, which current GCC rejects); elsewhere they are stubbed out.
if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64|amd64|i[3-6]86|x86)$")
    set(_simd_dir "${_idt3_cmp}/cmp_core/source")
    target_sources(idt3_cmp_core PRIVATE
        "${_simd_dir}/core_simd_sse.cpp"
        "${_simd_dir}/core_simd_avx.cpp"
        "${_simd_dir}/core_simd_avx512.cpp")
    if(MSVC)
        # SSE4.2 intrinsics need no flag with MSVC on x86-64.
        set_source_files_properties("${_simd_dir}/core_simd_avx.cpp" PROPERTIES COMPILE_OPTIONS "/arch:AVX2")
        set_source_files_properties("${_simd_dir}/core_simd_avx512.cpp" PROPERTIES COMPILE_OPTIONS "/arch:AVX512")
    else()
        set_source_files_properties("${_simd_dir}/core_simd_sse.cpp" PROPERTIES COMPILE_OPTIONS "-msse4.2")
        set_source_files_properties("${_simd_dir}/core_simd_avx.cpp" PROPERTIES COMPILE_OPTIONS "-mavx2;-mfma")
        set_source_files_properties("${_simd_dir}/core_simd_avx512.cpp" PROPERTIES COMPILE_OPTIONS "-mavx512f")
    endif()
else()
    target_sources(idt3_cmp_core PRIVATE "${CMAKE_CURRENT_LIST_DIR}/cmp_simd_stubs.cpp")
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
