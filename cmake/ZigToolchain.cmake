# CMake toolchain file that drives `zig cc` / `zig c++` as a cross compiler.
#
#   cmake -S . -B build/windows-x86_64 -G Ninja \
#         --toolchain cmake/ZigToolchain.cmake -DIDT3_TARGET=windows-x86_64
#
# IDT3_TARGET is one of the platforms in targets.json. zig ships the headers,
# the libc implementations or stubs and the runtime libraries for every target
# it supports, so the same host produces binaries for all of them without any
# SDK being installed:
#
#   - Linux targets link musl statically, so the binaries run on any
#     distribution regardless of its libc.
#   - Windows targets use the mingw-w64 CRT that zig bundles. That keeps every
#     input pinned by the zig version (the MSVC CRT would have to be fetched
#     from Microsoft, whose packages move underneath a fixed manifest).
#   - macOS targets link against zig's bundled libSystem stubs.
#
# Release builds are reproducible byte for byte: no debug information, no
# absolute paths, and zig's linkers emit no timestamps or random IDs.

# CMake includes a toolchain file once per language and once more for every
# try_compile() project it spawns. Everything below is therefore written to be
# idempotent, and the variables the file depends on are forwarded explicitly.
list(APPEND CMAKE_TRY_COMPILE_PLATFORM_VARIABLES
     IDT3_TARGET
     IDT3_ZIG_EXECUTABLE
     IDT3_LLVM_AR_EXECUTABLE
     IDT3_LLVM_RANLIB_EXECUTABLE)

if(NOT IDT3_TARGET)
    message(FATAL_ERROR
            "IDT3_TARGET is not set. Pass -DIDT3_TARGET=<one of the "
            "platforms in cmake/targets.json>.")
endif()

# The platform table lives in targets.json because dev.py reads the very same
# values to name and describe the artifacts; one file both sides parse cannot
# drift.
get_filename_component(_idt3_root "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
file(READ "${CMAKE_CURRENT_LIST_DIR}/targets.json" _idt3_targets)

string(JSON _idt3_platform ERROR_VARIABLE _idt3_json_error
       GET "${_idt3_targets}" "platforms" "${IDT3_TARGET}")
if(NOT _idt3_json_error STREQUAL "NOTFOUND")
    message(FATAL_ERROR "Unsupported IDT3_TARGET '${IDT3_TARGET}': ${_idt3_json_error}")
endif()

string(JSON CMAKE_SYSTEM_NAME GET "${_idt3_platform}" "system")
string(JSON CMAKE_SYSTEM_PROCESSOR GET "${_idt3_platform}" "processor")
string(JSON _idt3_triple GET "${_idt3_platform}" "triple")

# The instruction set the generated code may assume. zig already defaults to
# `baseline` for a cross target, but a native build would otherwise default to
# the build machine's CPU, which makes the output both host dependent and
# possibly unusable elsewhere. The CMP_Core SIMD variants are compiled with
# their own per-file ISA flags and are only entered after a runtime check.
string(JSON _idt3_cpu GET "${_idt3_targets}" "mcpu")

# Optional per-platform flags from targets.json.
function(_idt3_field out_var json field)
    string(JSON _value ERROR_VARIABLE _error GET "${json}" "${field}")
    if(NOT _error STREQUAL "NOTFOUND")
        set(_value "")
    endif()
    set(${out_var} "${_value}" PARENT_SCOPE)
endfunction()
_idt3_field(_idt3_compile_flags "${_idt3_platform}" "compile-flags")
_idt3_field(_idt3_link_flags    "${_idt3_platform}" "link-flags")

# On Windows conda-forge only provides `zig` as zig.bat/zig.cmd wrappers, which
# find_program does not consider (it only tries .com and .exe). Those wrappers
# merely forward to the triplet prefixed executable, so that is looked for too;
# using it directly also keeps batch files out of the generated shims.
find_program(IDT3_ZIG_EXECUTABLE
             NAMES zig x86_64-w64-mingw32-zig aarch64-w64-mingw32-zig
             REQUIRED
             DOC "zig compiler used as the cross toolchain")

# Location of zig's bundled library tree (headers, libc sources, compiler-rt).
# `zig env` prints a path relative to its working directory when it can, hence
# the explicit working directory plus the matching ABSOLUTE base below.
execute_process(
    COMMAND "${IDT3_ZIG_EXECUTABLE}" env
    WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    OUTPUT_VARIABLE _idt3_zig_env
    OUTPUT_STRIP_TRAILING_WHITESPACE
    COMMAND_ERROR_IS_FATAL ANY)
# Matches both the ZON output of zig >= 0.14 and the older JSON output.
string(REGEX MATCH "lib_dir\"?[ \t]*[=:][ \t]*\"([^\"]*)\"" _idt3_unused "${_idt3_zig_env}")
get_filename_component(_idt3_zig_lib_dir "${CMAKE_MATCH_1}" ABSOLUTE
                       BASE_DIR "${CMAKE_BINARY_DIR}")

# CMake wants a single executable per tool, but every zig tool is a sub-command
# of one binary. Small generated shim scripts bridge that gap, and they are also
# where the target triple is pinned so that it applies to compiling and linking
# alike.
set(_idt3_bin_dir "${CMAKE_BINARY_DIR}/zig-toolchain")

if(CMAKE_HOST_APPLE AND CMAKE_SYSTEM_NAME STREQUAL "Darwin")
    # Conda-forge's zig probes ../../lib relative to ZIG_LIB_DIR for a shared
    # libc++. On a matching host architecture that picks up pixi's runtime and
    # gives the binary an @rpath dependency that cross-host builds do not
    # have. A separate library root keeps that probe away from host libraries
    # and makes zig use its bundled static runtime. The entries are linked one
    # by one: linking the root itself would make ../.. resolve into the host
    # prefix again.
    set(_idt3_static_lib_dir "${_idt3_bin_dir}/runtime/lib/zig")
    file(MAKE_DIRECTORY "${_idt3_static_lib_dir}")
    file(GLOB _idt3_lib_entries RELATIVE "${_idt3_zig_lib_dir}" "${_idt3_zig_lib_dir}/*")
    foreach(_idt3_entry IN LISTS _idt3_lib_entries)
        file(CREATE_LINK "${_idt3_zig_lib_dir}/${_idt3_entry}"
                         "${_idt3_static_lib_dir}/${_idt3_entry}" SYMBOLIC)
    endforeach()
    set(IDT3_WRAPPER_ENV "export ZIG_LIB_DIR=\"${_idt3_static_lib_dir}\"")
else()
    set(IDT3_WRAPPER_ENV "")
endif()

function(_idt3_wrapper out_var name)
    # Everything after `name` becomes the fixed argument list of the shim.
    list(JOIN ARGN " " IDT3_WRAPPER_ARGS)

    if(CMAKE_HOST_WIN32)
        set(_template "${CMAKE_CURRENT_LIST_DIR}/zig-wrapper.bat.in")
        set(_output "${_idt3_bin_dir}/${name}.bat")
    else()
        set(_template "${CMAKE_CURRENT_LIST_DIR}/zig-wrapper.sh.in")
        set(_output "${_idt3_bin_dir}/${name}")
    endif()

    # configure_file() rewrites the shim only when its content actually
    # changes, so re-running CMake does not invalidate the whole build.
    configure_file("${_template}" "${_output}" @ONLY
                   FILE_PERMISSIONS
                   OWNER_READ OWNER_WRITE OWNER_EXECUTE
                   GROUP_READ GROUP_EXECUTE
                   WORLD_READ WORLD_EXECUTE)

    set(${out_var} "${_output}" PARENT_SCOPE)
endfunction()

_idt3_wrapper(_idt3_cc  zig-cc  cc  -target ${_idt3_triple} -mcpu=${_idt3_cpu})
_idt3_wrapper(_idt3_cxx zig-cxx c++ -target ${_idt3_triple} -mcpu=${_idt3_cpu})
if(CMAKE_HOST_APPLE)
    # zig's bundled archiver fails to create new archives on macOS 26, so the
    # standalone LLVM tools pixi provides are used on these hosts. Apple's own
    # tools cannot index the ELF and COFF objects produced for other targets.
    find_program(IDT3_LLVM_AR_EXECUTABLE NAMES llvm-ar REQUIRED)
    find_program(IDT3_LLVM_RANLIB_EXECUTABLE NAMES llvm-ranlib REQUIRED)
    set(_idt3_ar "${IDT3_LLVM_AR_EXECUTABLE}")
    set(_idt3_ranlib "${IDT3_LLVM_RANLIB_EXECUTABLE}")
else()
    _idt3_wrapper(_idt3_ar     zig-ar     ar)
    _idt3_wrapper(_idt3_ranlib zig-ranlib ranlib)
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "Windows")
    # CMake enables the RC language for Windows targets and insists on a real
    # program even when, as here, there is not a single .rc file to compile.
    _idt3_wrapper(_idt3_rc zig-rc rc)
    set(CMAKE_RC_COMPILER "${_idt3_rc}" CACHE FILEPATH "zig rc" FORCE)
endif()

set(CMAKE_C_COMPILER "${_idt3_cc}")
set(CMAKE_CXX_COMPILER "${_idt3_cxx}")

# zig reports itself as clang, so CMake would otherwise go looking for a
# stand-alone llvm-ar / llvm-ranlib next to it and not find one. Both zig's
# archiver and llvm-ar write deterministic archives (zeroed timestamps, uids
# and modes) by default.
set(CMAKE_AR "${_idt3_ar}" CACHE FILEPATH "archiver" FORCE)
set(CMAKE_RANLIB "${_idt3_ranlib}" CACHE FILEPATH "ranlib" FORCE)
set(CMAKE_C_COMPILER_AR "${_idt3_ar}" CACHE FILEPATH "archiver" FORCE)
set(CMAKE_CXX_COMPILER_AR "${_idt3_ar}" CACHE FILEPATH "archiver" FORCE)
set(CMAKE_C_COMPILER_RANLIB "${_idt3_ranlib}" CACHE FILEPATH "ranlib" FORCE)
set(CMAKE_CXX_COMPILER_RANLIB "${_idt3_ranlib}" CACHE FILEPATH "ranlib" FORCE)

if(CMAKE_SYSTEM_NAME STREQUAL "Darwin")
    # zig carries its own copy of the macOS libc headers and of the libSystem
    # stub library, so there is no Xcode SDK to point CMake at. Clearing these
    # also stops CMake from shelling out to `xcrun`, which only exists on macOS.
    set(CMAKE_OSX_SYSROOT "" CACHE INTERNAL "handled by zig")
    set(CMAKE_OSX_DEPLOYMENT_TARGET "" CACHE INTERNAL "handled by zig")
    set(CMAKE_OSX_ARCHITECTURES "" CACHE INTERNAL "handled by zig")

    # Some zig distributions (the conda-forge one among them) wrap the compiler
    # and force `-fuse-ld=lld`. That bypasses the code path where zig hands its
    # bundled libSystem.tbd to the linker, and the link then fails with
    # "library not found for -lSystem". Putting the stub directory on the
    # library search path fixes that and is a no-op otherwise.
    string(APPEND _idt3_link_flags " -L${_idt3_zig_lib_dir}/libc/darwin")
endif()

# zig enables UBSan in debug builds and satisfies it by linking its own
# runtime. Trapping keeps the checks but drops that runtime, which does not
# exist for every libc. Release builds, where zig turns the checks off, are
# unaffected.
string(APPEND _idt3_compile_flags " -fsanitize-trap=undefined")

# Reproducibility: paths that end up in the output (__FILE__ and, in debug
# builds, the DWARF file names) are rewritten to be relative, so the checkout
# location and the pixi environment location do not matter. The more specific
# zig library mapping comes last because the debug-info mapping lets the last
# matching option win (the macro mapping prefers the longest match anyway),
# and the pixi environment usually lives inside the checkout. The compilation
# directory, the build directory, is recorded as `.`.
string(APPEND _idt3_compile_flags
       " -ffile-prefix-map=${_idt3_root}=."
       " -ffile-prefix-map=${_idt3_zig_lib_dir}=zig-lib"
       " -fdebug-compilation-dir=.")

# Plain assignment, not append: this file is re-read once per language and
# again for every try_compile, and appending would stack the flags up.
set(CMAKE_C_FLAGS_INIT "${_idt3_compile_flags}")
set(CMAKE_CXX_FLAGS_INIT "${_idt3_compile_flags}")
foreach(_idt3_kind EXE SHARED MODULE)
    set(CMAKE_${_idt3_kind}_LINKER_FLAGS_INIT "${_idt3_link_flags}")
    # zig cc emits debug information even for optimized builds unless told
    # otherwise. Release artifacts are shipped, so they carry no symbols
    # either; that also keeps zig's bundled runtime libraries' debug info out.
    set(CMAKE_${_idt3_kind}_LINKER_FLAGS_RELEASE_INIT "-s")
endforeach()
set(CMAKE_C_FLAGS_RELEASE_INIT "-g0")
set(CMAKE_CXX_FLAGS_RELEASE_INIT "-g0")

# There is no target sysroot on disk - zig resolves the target headers and
# libraries internally. Pointing CMAKE_FIND_ROOT_PATH at an empty directory
# makes the ONLY modes below actually bite, so that a cross build cannot
# accidentally pick up a host library from /usr while host tools such as ninja
# or python keep resolving normally.
file(MAKE_DIRECTORY "${CMAKE_BINARY_DIR}/zig-sysroot")
set(CMAKE_FIND_ROOT_PATH "${CMAKE_BINARY_DIR}/zig-sysroot")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM BEFORE)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
