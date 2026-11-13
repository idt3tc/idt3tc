//! Compiles the vendored Compressonator CMP_Core (BC1-BC7 block encoders)
//! into a static library linked into idt3tc.
//!
//! The vendored files in third_party/compressonator are pristine upstream
//! copies. They are copied into OUT_DIR with the fixes below applied, and
//! compiled from there together with the replacements in cmp/.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Directories of the vendored tree that are copied and compiled. Whole
/// directories are copied because the sources include their headers with
/// quotes, which always finds the sibling file first.
const CMP_DIRS: &[&str] = &[
    "cmp_core/shaders",
    "cmp_core/source",
    "applications/_libs/cmp_math",
];

/// Exact text substitutions (file, old, new). `old` must occur, so a version
/// bump that moves the code fails loudly instead of silently dropping a fix.
const CMP_PATCHES: &[(&str, &str, &str)] = &[
    // pack_index() shifts a 32-bit value by up to 60 bits. GCC on x86 happens
    // to produce what was meant (the hardware masks the count to 5 bits);
    // clang treats the path as unreachable and the encoder traps on the first
    // BC7 mode 4/5 block. k / 8 already selects the word, so masking is what
    // was intended.
    (
        "cmp_core/shaders/bc7_encode_kernel.cpp",
        "(CGV_UINT32)(src_index[k] & 0x0F) << shift;",
        "(CGV_UINT32)(src_index[k] & 0x0F) << (shift & 31);",
    ),
    // The BC3/BC4/BC5 alpha index packing shifts signed 3-bit indices into
    // bit 31 and beyond, which overflows int; the target words are unsigned.
    (
        "cmp_core/shaders/bcn_common_kernel.h",
        "cmpBlock.x |= (pcIndices[i] << (16 + (i * 3)));",
        "cmpBlock.x |= ((CGU_UINT32)pcIndices[i] << (16 + (i * 3)));",
    ),
    (
        "cmp_core/shaders/bcn_common_kernel.h",
        "cmpBlock.x |= (pcIndices[5] << 31);",
        "cmpBlock.x |= ((CGU_UINT32)pcIndices[5] << 31);",
    ),
    (
        "cmp_core/shaders/bcn_common_kernel.h",
        "cmpBlock.y |= (pcIndices[i] << (i * 3 - 16));",
        "cmpBlock.y |= ((CGU_UINT32)pcIndices[i] << (i * 3 - 16));",
    ),
];

/// Upstream sources compiled into the library (relative to the tree root).
/// Upstream's CPUID based SIMD dispatch is replaced by cmp/ so that every
/// platform runs the same scalar code; the SIMD variants of the BC1
/// endpoint search are therefore never compiled, only stubbed for the linker.
const CMP_SOURCES: &[&str] = &[
    "cmp_core/shaders/bc1_encode_kernel.cpp",
    "cmp_core/shaders/bc2_encode_kernel.cpp",
    "cmp_core/shaders/bc3_encode_kernel.cpp",
    "cmp_core/shaders/bc4_encode_kernel.cpp",
    "cmp_core/shaders/bc5_encode_kernel.cpp",
    "cmp_core/shaders/bc6_encode_kernel.cpp",
    "cmp_core/shaders/bc7_encode_kernel.cpp",
    "cmp_core/source/cmp_core.cpp",
    "applications/_libs/cmp_math/cmp_math_common.cpp",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let upstream = manifest.join("third_party/compressonator");
    let tree = out.join("compressonator");

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=third_party/compressonator");
    println!("cargo::rerun-if-changed=cmp");

    prepare_tree(&upstream, &tree);

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++14")
        .warnings(false)
        .pic(true)
        .define("NDEBUG", None)
        // Always optimized, independent of the cargo profile: the encoder
        // output of an -O0 build differs in rare blocks from the optimized
        // one, and tests must see the same bytes as release builds.
        .opt_level(3)
        .include(tree.join("cmp_core/source"))
        .include(tree.join("cmp_core/shaders"))
        .include(tree.join("applications/_libs/cmp_math"));
    for src in CMP_SOURCES {
        build.file(tree.join(src));
    }
    build.file(manifest.join("cmp/cmp_cpu_extensions.cpp"));
    build.file(manifest.join("cmp/cmp_simd_stubs.cpp"));
    build.file(manifest.join("cmp/cmp_capi.cpp"));

    let msvc_like = target_env == "msvc";
    // Same defines as upstream's CMP_Core target.
    if target_os != "windows" {
        build.define("_LINUX", None).define("ASPM_GPU", None);
    }

    // Flags are passed in the clang driver spelling; clang-cl (used by
    // cargo-xwin for the MSVC targets) needs them wrapped in /clang:.
    let clang_flag = |b: &mut cc::Build, flag: &str| {
        if msvc_like {
            b.flag(format!("/clang:{flag}"));
        } else {
            b.flag(flag);
        }
    };
    // No implicit FMA contraction: identical float results on every CPU.
    clang_flag(&mut build, "-ffp-contract=off");
    // Build paths must not leak into the objects (reproducible builds from
    // any checkout location).
    for (from, to) in [(&out, "/out"), (&manifest, "/src")] {
        clang_flag(&mut build, &format!("-ffile-prefix-map={}={to}", from.display()));
    }

    // On Windows x86, cmp_floor() uses the SSE4.1 _mm_floor_ss intrinsic,
    // which MSVC accepts without any flag but clang only for an SSE4.1
    // target. Rather than raising the CPU baseline of the whole library, an
    // SSE2 equivalent is put in place before the code is seen.
    if target_os == "windows" && (target_arch == "x86_64" || target_arch == "x86") {
        let compat = manifest.join("cmp/cmp_win_x86_compat.h");
        if msvc_like {
            build.flag(format!("/FI{}", compat.display()));
        } else {
            build.flag("-include").flag(compat.to_str().unwrap());
        }
    }

    build.compile("idt3tc_cmp_core");
}

/// Copies the vendored directories into `tree`, applying CMP_PATCHES.
/// Files are only rewritten when their content changes, so rebuilding does
/// not touch timestamps.
fn prepare_tree(upstream: &Path, tree: &Path) {
    let mut applied = vec![false; CMP_PATCHES.len()];
    for dir in CMP_DIRS {
        let src_dir = upstream.join(dir);
        let entries = fs::read_dir(&src_dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", src_dir.display()));
        let mut names: Vec<_> = entries.map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
        names.sort();
        for path in names {
            let rel = format!("{dir}/{}", path.file_name().unwrap().to_str().unwrap());
            let mut content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            for (i, (file, old, new)) in CMP_PATCHES.iter().enumerate() {
                if *file == rel {
                    assert!(
                        content.contains(old),
                        "Compressonator patch not applicable to {rel}: '{old}'"
                    );
                    content = content.replace(old, new);
                    applied[i] = true;
                }
            }
            let dest = tree.join(&rel);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            if fs::read_to_string(&dest).ok().as_deref() != Some(content.as_str()) {
                fs::write(&dest, content).unwrap();
            }
        }
    }
    for (i, done) in applied.iter().enumerate() {
        assert!(*done, "Compressonator patch target missing: {}", CMP_PATCHES[i].0);
    }
}
