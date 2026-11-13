# idt3tc - idTech 3 provisioning toolchain

Command line tools for preparing assets for idTech 3 engines (Quake III Arena, ioquake3, Wolfenstein: Enemy Territory, Jedi Academy, ...): maps, mods, texture packs and pk3s.

Written in Rust, built with cargo. Subcommands:

| command    | purpose                                                                 |
|------------|-------------------------------------------------------------------------|
| `idt3tc img` | convert and process images: TGA, DDS (BC1-BC7 via AMD Compressonator), PNG, JPG, BMP |

## Building

### Release binaries for every platform

Rust comes from [rustup](https://rustup.rs); `rust-toolchain.toml` pins the exact version together with the cross targets; `rustup toolchain install` in the checkout installs all of them. [pixi](https://pixi.sh) provides the rest: zig and [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) for Linux and macOS, clang-cl, lld-link and llvm-lib for Windows, and Python. Every platform is cross compiled from any host pixi supports (Linux, macOS and Windows, x86_64 and arm64):

```sh
pixi run dist             # all platforms, collected into ./dist
pixi run test             # run the tests on the host (debug assertions, overflow checks)
pixi run test --release   # ... with the release profile
pixi run repro            # build everything twice and compare byte by byte
pixi run dev --help       # all dev.py subcommands
```

`./dev.py` works the same way and enters the pixi environment by itself when the tools are not on `PATH`. `./dev.py build -t windows-x86_64` builds a single platform into `target/<rust target>/release`, `./dev.py build -a` builds all of them, and `./dev.py targets` lists them:

| platform         | Rust target                  | built with      | artifact                    |
|------------------|------------------------------|-----------------|-----------------------------|
| `linux-x86_64`   | `x86_64-unknown-linux-musl`  | cargo-zigbuild  | `idt3tc-linux-x86_64`       |
| `linux-arm64`    | `aarch64-unknown-linux-musl` | cargo-zigbuild  | `idt3tc-linux-arm64`        |
| `macos-x86_64`   | `x86_64-apple-darwin`        | cargo-zigbuild  | `idt3tc-macos-x86_64`       |
| `macos-arm64`    | `aarch64-apple-darwin`       | cargo-zigbuild  | `idt3tc-macos-arm64`        |
| `windows-x86_64` | `x86_64-pc-windows-msvc`     | cargo-xwin      | `idt3tc-windows-x86_64.exe` |
| `windows-arm64`  | `aarch64-pc-windows-msvc`    | cargo-xwin      | `idt3tc-windows-arm64.exe`  |

The platform table lives in `PLATFORMS` in `dev.py`. The macOS builds target macOS 10.13 (x86_64) and 11.0 (arm64). Expect a warning that `xcrun --sdk macosx --show-sdk-path` failed when building them off a Mac; zig supplies the `libSystem` stubs instead of an SDK.

[cargo-xwin](https://github.com/rust-cross/cargo-xwin) is not packaged on conda-forge, so `dev.py` installs the pinned version into `.cache/tools` with `cargo install --locked`. The first Windows build then downloads the pinned MSVC CRT and Windows SDK (the versions are set in `dev.py`) into `.cache/xwin`.

The Linux binaries are fully static, the macOS ones only link `libSystem`, and the Windows ones link the C runtime statically and only depend on DLLs that ship with Windows 10 and later (`KERNEL32`, `ntdll`, `bcryptprimitives` and an API set), not on the Visual C++ redistributable. All target the baseline instruction set of their CPU.

`dist` also writes `SHA256SUMS`. With `--url-base URL` it additionally writes `idt3tc.multitool.json`, a [rules_multitool](https://github.com/theoremlp/rules_multitool) lock file for binaries published under `URL`. The platform names are rules_multitool's `os`/`cpu` values.

### Dependencies

- Crates, pinned with checksums in `Cargo.lock`: [image](https://crates.io/crates/image) for decoding PNG, JPEG, BMP, GIF, HDR, PNM and TGA (only these decoders are enabled), [png](https://crates.io/crates/png), [rayon](https://crates.io/crates/rayon) for the parallel runs and [libm](https://crates.io/crates/libm) for math functions that give the same results on every platform. The DDS and PSD readers are part of idt3tc.
- [Compressonator](https://github.com/GPUOpen-Tools/compressonator) V4.6.0: the `CMP_Core` BC1-BC7 block encoders and decoders, vendored unmodified in `third_party/compressonator` (see its README there). `build.rs` compiles them with the C API shim in `cmp/` and applies the local fixes for undefined behaviour at build time.

### Reproducibility

Release builds are bit-for-bit reproducible: the same commit with the same pinned toolchains (Rust from `rust-toolchain.toml`, the pixi environment from `pixi.lock`, cargo-xwin and the MSVC CRT/SDK from `dev.py`) produces identical binaries regardless of the checkout path or the state of the build caches. Neither timestamps, build paths nor debug info end up in the binaries:

- Rust paths are remapped with `--remap-path-prefix`, the C++ ones with `-ffile-prefix-map`.
- The Windows binaries are linked without a PDB and with `/Brepro`, which replaces the link time in the PE header with a content hash (`.cargo/config.toml`).
- The macOS binaries are linked with `-Wl,-S`, so the Mach-O UUID does not depend on object paths.

`pixi run repro` checks this by building a second time in a copy of the tree at another path, with its own target directory and empty zig caches.

The image output is reproducible too: floating point contraction is disabled for the C++ code, Compressonator's runtime SIMD dispatch is replaced by its portable code, and math functions come from libm instead of the platform, so the same input and options give the same bytes on every platform and CPU.

### CI and releases

The `CI` workflow builds and tests every platform on `ubuntu-latest` and runs the reproducibility check. Started manually with `release` checked, it also publishes the binaries, `SHA256SUMS` and `idt3tc.multitool.json` as GitHub release `v<version>`, using the version in `Cargo.toml`. Assets are uploaded into a draft that is only published once all of them arrived intact, and deleted otherwise. An existing release is never overwritten.

### With plain cargo

On Linux and macOS, plain cargo with the host's GCC or clang works too, but without the guarantees above:

```sh
cargo build --release   # target/release/idt3tc
cargo test
```

## `idt3tc img`

```
idt3tc img [OPTIONS] [INPUT...] --OP [ARG] [key=value...] [--OP ...]
```

Operations run **in the order given**, once per input. Every operation has an optional argument and `key=value` options; flag options can be given as a bare key (`rle` is the same as `rle=1`). Unknown or non-applicable keys are errors, not silently ignored.

```sh
idt3tc img --help            # overview of all operations
idt3tc img --help resize     # details and options of one operation
idt3tc img --help all        # everything
```

Global options: `-j N` (threads, default all cores), `-r` (recurse into directory inputs), `-k` (keep going after errors), `-q`, `-v`.

### Inputs and outputs

Inputs are image files (PNG, JPEG, TGA, BMP, PSD, GIF, HDR, PNM, DDS) or directories. `--save` may appear several times in one pipeline, so a single run can produce several variants. Output paths support placeholders:

| placeholder | value                                                     |
|-------------|-----------------------------------------------------------|
| `{name}`    | input file name without extension                         |
| `{ext}`     | input extension                                           |
| `{dir}`     | input directory                                           |
| `{rel}`     | path below a directory input, without extension           |
| `{index}`   | 0-based input number                                      |
| `{w}` `{h}` | current image size                                        |

With several inputs, each `--save` path must contain an input-dependent placeholder, otherwise the run is rejected (it would overwrite the same file).

### Operations

- **Input & output:** `--new`, `--load`, `--save`/`-o`, `--info`, `--channel-from`, `--composite`
- **Geometry:** `--resize`, `--pot`, `--crop`, `--canvas`, `--pad`, `--trim`, `--rotate`, `--flip`, `--transpose`, `--pan`, `--tile`
- **Color & alpha:** `--gamma`, `--brightness`, `--contrast`, `--levels`, `--saturation`, `--hue`, `--grayscale`, `--invert`, `--tint`, `--fill`, `--colorkey`, `--alpha`, `--flatten`, `--swizzle`
- **Filters:** `--blur`, `--sharpen`, `--normalmap`

Resampling operations accept `filter=` (nearest, box, triangle, hermite, bspline, mitchell, catmullrom, gaussian, lanczos2, lanczos3, kaiser), `edge=` (clamp, repeat, mirror, fill; use `repeat` for tiling textures), `linear` (gamma-correct filtering), `premultiply` and `blur=`.

Colors are `#rrggbb[aa]`, `rrggbb[aa]`, `#rgb[a]`, `r,g,b[,a]` (0-255) or a name. Quote `#` colors in the shell when they start a word (`--fill '#ff0000'`), or leave the `#` out.

### Output formats

**TGA** (`.tga`): defaults match the idTech 3 loader: uncompressed, bottom-up rows (ioquake3 ignores the top-down flag), 32 bit with alpha else 24 bit. Options: `bpp=auto|8|24|32`, `rle`, `origin=bottom|top`. RLE packets never cross scanlines. 8-bit RLE (type 11) is not supported by the engine, so `idt3tc` warns about it.

**DDS** (`.dds`), for engines with DDS support:

| `codec=`       | format                                | header      |
|----------------|---------------------------------------|-------------|
| `auto`         | `bc1` when opaque, otherwise `bc3`    |             |
| `bc1`/`dxt1`   | opaque RGB, 4 bpp                     | legacy      |
| `bc1a`/`dxt1a` | RGB + 1-bit alpha, 4 bpp              | legacy      |
| `bc2`/`dxt3`   | RGB + 4-bit alpha, 8 bpp              | legacy      |
| `bc3`/`dxt5`   | RGB + interpolated alpha, 8 bpp       | legacy      |
| `bc4`/`ati1`   | single channel                        | legacy      |
| `bc5`/`ati2`   | two channels (normal maps)            | legacy      |
| `bc7`          | high quality RGBA, 8 bpp              | DX10        |
| `rgba8`, `bgra8`, `bgrx8`, `bgr8`, `l8`, `a8` | uncompressed | legacy |

Further DDS options:

- `mips=full|none|N` and `mip-min=`.
- `mip-filter=` and the other `mip-*` resampling options. The default is a box filter with `repeat` edges, and every level is filtered from the base image.
- `mip-coverage=REF` keeps alpha-test coverage stable across mips, so foliage and grates don't thin out at a distance.
- Encoder knobs: `quality=`, `refine=`, `weights=perceptual|R,G,B`, `alpha-threshold=`.
- Header control: `dx10=auto|0|1` and `srgb`.

**PNG/JPG/BMP**: `compression=`, `quality=`, `channels=`.

### Examples

```sh
# Photo to a power-of-two idTech 3 texture
idt3tc img photo.jpg --pot --save textures/mymap/wall.tga

# A whole folder at half size, RLE TGAs mirroring the folder structure
idt3tc img src/ -r --resize 50% filter=lanczos3 --save out/{rel}.tga rle

# Alpha-tested grate as DXT1 with 1-bit alpha and coverage-preserving mips
idt3tc img grate.png --pot max=512 --save grate.dds codec=bc1a mip-coverage=0.5

# Height map to a BC5 normal map
idt3tc img height.png --normalmap strength=4 --save wall_n.dds codec=bc5

# Check seams of a tiling texture: offset by half its size
idt3tc img tile.tga --pan 50%,50% --save tile_seams.png

# Several variants of one source in one run
idt3tc img logo.png --trim --resize 240x240 fit=contain --canvas 256x256 --save logo.tga \
                  --resize 128x128 --save logo_small.tga --save logo.dds codec=bc7

# Sprite with a magenta background to a TGA with alpha
idt3tc img sprite.bmp --colorkey ff00ff fuzz=0.05 soft=0.05 --save sprite.tga
```

Exit codes: `0` success, `1` processing error, `2` usage error.
