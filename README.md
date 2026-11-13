# idt3 - idTech 3 provisioning toolchain

Command line tools for preparing assets for idTech 3 engines (Quake III Arena, ioquake3, Wolfenstein: Enemy Territory, Jedi Academy, ...): maps, mods, texture packs and pk3s.

Written in C++11, built with CMake. Subcommands:

| command    | purpose                                                                 |
|------------|-------------------------------------------------------------------------|
| `idt3 img` | convert and process images: TGA, DDS (BC1-BC7 via AMD Compressonator), PNG, JPG, BMP |

## Building

```sh
cmake -S . -B build -G Ninja
cmake --build build
ctest --test-dir build
```

CMake downloads two pinned dependencies (hash-verified) at configure time:

- [Compressonator](https://github.com/GPUOpen-Tools/compressonator) V4.6.0: only the `CMP_Core` block encoders/decoders are compiled, into a static library.
- [stb](https://github.com/nothings/stb): `stb_image` (decoding) and `stb_image_write` (PNG/JPG/BMP).

For offline builds, point CMake at local checkouts:

```sh
cmake -S . -B build -DFETCHCONTENT_SOURCE_DIR_COMPRESSONATOR=/path/to/compressonator \
                    -DFETCHCONTENT_SOURCE_DIR_STB=/path/to/stb
```

Options: `IDT3_BUILD_TESTS` (default `ON`), `IDT3_WARNINGS_AS_ERRORS` (default `OFF`).

## `idt3 img`

```
idt3 img [OPTIONS] [INPUT...] --OP [ARG] [key=value...] [--OP ...]
```

Operations run **in the order given**, once per input. Every operation has an optional argument and `key=value` options; flag options can be given as a bare key (`rle` is the same as `rle=1`). Unknown or non-applicable keys are errors, not silently ignored.

```sh
idt3 img --help            # overview of all operations
idt3 img --help resize     # details and options of one operation
idt3 img --help all        # everything
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

**TGA** (`.tga`): defaults match the idTech 3 loader: uncompressed, bottom-up rows (ioquake3 ignores the top-down flag), 32 bit with alpha else 24 bit. Options: `bpp=auto|8|24|32`, `rle`, `origin=bottom|top`. RLE packets never cross scanlines. 8-bit RLE (type 11) is not supported by the engine, so `idt3` warns about it.

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
idt3 img photo.jpg --pot --save textures/mymap/wall.tga

# A whole folder at half size, RLE TGAs mirroring the folder structure
idt3 img src/ -r --resize 50% filter=lanczos3 --save out/{rel}.tga rle

# Alpha-tested grate as DXT1 with 1-bit alpha and coverage-preserving mips
idt3 img grate.png --pot max=512 --save grate.dds codec=bc1a mip-coverage=0.5

# Height map to a BC5 normal map
idt3 img height.png --normalmap strength=4 --save wall_n.dds codec=bc5

# Check seams of a tiling texture: offset by half its size
idt3 img tile.tga --pan 50%,50% --save tile_seams.png

# Several variants of one source in one run
idt3 img logo.png --trim --resize 240x240 fit=contain --canvas 256x256 --save logo.tga \
                  --resize 128x128 --save logo_small.tga --save logo.dds codec=bc7

# Sprite with a magenta background to a TGA with alpha
idt3 img sprite.bmp --colorkey ff00ff fuzz=0.05 soft=0.05 --save sprite.tga
```

Exit codes: `0` success, `1` processing error, `2` usage error.
