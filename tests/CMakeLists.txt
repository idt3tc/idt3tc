# Unit tests (framework-less, see test.h) and CLI tests driven by CTest.

add_executable(idt3_tests
    test_main.cpp
    test_parse.cpp
    test_image_ops.cpp
    test_io.cpp)
target_link_libraries(idt3_tests PRIVATE idt3_core)
target_include_directories(idt3_tests PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}")
add_test(NAME unit COMMAND idt3_tests)

# --- CLI ----------------------------------------------------------------------
set(_work "${CMAKE_CURRENT_BINARY_DIR}/cli")
file(MAKE_DIRECTORY "${_work}")

# Successful invocations; FIXTURES_* orders tests that read files written
# by earlier ones (also under ctest -j).
function(idt3_cli_test name)
    cmake_parse_arguments(T "" "SETUP;REQUIRES;EXPECT" "" ${ARGN})
    add_test(NAME cli_${name} COMMAND idt3 ${T_UNPARSED_ARGUMENTS} WORKING_DIRECTORY "${_work}")
    if(T_SETUP)
        set_tests_properties(cli_${name} PROPERTIES FIXTURES_SETUP ${T_SETUP})
    endif()
    if(T_REQUIRES)
        set_tests_properties(cli_${name} PROPERTIES FIXTURES_REQUIRED ${T_REQUIRES})
    endif()
    if(T_EXPECT)
        set_tests_properties(cli_${name} PROPERTIES PASS_REGULAR_EXPRESSION "${T_EXPECT}")
    endif()
endfunction()

# Invocations that must be rejected; the expected message is matched
# (ctest ignores the exit code when a pass regex is set).
function(idt3_cli_error name expect)
    idt3_cli_test(${name} ${ARGN} EXPECT "${expect}")
endfunction()

idt3_cli_test(version --version EXPECT "idt3 [0-9]+\\.[0-9]+")
idt3_cli_test(help img --help EXPECT "usage: idt3 img")
idt3_cli_test(help_op img --help save EXPECT "codec=auto")
idt3_cli_test(help_all img --help all EXPECT "--normalmap")

idt3_cli_test(generate
    img --new 300x200 "color=#4080c0" --rotate 20 bg=transparent --save src.png
        --flip h --save set/a.png --tile 2 --save set/sub/b.png
    SETUP cli_src)
idt3_cli_test(tga img src.png --pot --save out/{name}.tga rle
    REQUIRES cli_src EXPECT "wrote out/src.tga \\(256x256, tga 32-bit rle")
idt3_cli_test(dds img src.png --pot max=128 --save out/{name}_{w}.dds codec=bc1a mip-coverage=0.5
    REQUIRES cli_src EXPECT "wrote out/src_128.dds \\(128x128, dds bc1a/dxt1a, 8 mips")
idt3_cli_test(dds_bc7 img src.png --resize 64x64 --save out/bc7.dds codec=bc7 quality=0.05 srgb
    REQUIRES cli_src EXPECT "bc7, 7 mips, dx10")
idt3_cli_test(batch img set -r --resize 32x32 --save batch/{rel}.tga
    REQUIRES cli_src EXPECT "wrote batch/sub/b.tga")
idt3_cli_test(info img src.png --info REQUIRES cli_src EXPECT "src.png: 351x291 \\(not power of two\\), alpha: gradient")
idt3_cli_test(pipeline
    img --new 64x32 --fill ff0000 --pan 8,0 --crop 32x32+16+0 --levels 0,255,1.2 --hue 120
        --blur 1 edge=repeat --sharpen 0.5 --normalmap strength=3 --info
    EXPECT "64x32|32x32")

idt3_cli_error(unknown_op "did you mean '--resize'" img src.png --resiz 10x10)
idt3_cli_error(unknown_key "did you mean 'gravity'" img --new 8x8 --crop 4x4 gravty=c)
idt3_cli_error(foreign_key "option 'codec' is not applicable" img --new 8x8 --save x.tga codec=bc1)
idt3_cli_error(no_format "cannot derive the format" img --new 8x8 --save x.xyz)
idt3_cli_error(fixed_output "same for all" img set -r --save one.tga)
set_tests_properties(cli_fixed_output PROPERTIES FIXTURES_REQUIRED cli_src)
idt3_cli_error(missing_input "does not exist" img nope.png --info)
idt3_cli_error(no_image "no image loaded" img --resize 10x10)
idt3_cli_error(inputs_after_ops "inputs must come before" img --new 8x8 --info extra.png)
idt3_cli_error(bad_geometry "--resize" img --new 8x8 --resize 10y10)
