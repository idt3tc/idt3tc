# Notes for agents and contributors

## Inputs are untrusted and may be adversarial

idt3tc is expected to run on files nobody has vetted: textures pulled from
downloaded maps, mods and asset packs, user uploads processed by build or
server pipelines, and batch jobs over whole directory trees. Treat every
byte read from an input file as hostile. The same goes for directory
contents met during `-r` scans. A crafted file must never be able to crash
the process, corrupt memory, hang it, exhaust memory or disk, or make it
write outside the intended output location.

The goal for malformed or malicious input is a clean, specific error
message and exit code 1 (or a skipped input with `-k`). Crashes, aborts,
undefined behaviour and silently wrong output are not acceptable.

### Attack surface

- Image decoders: PNG, JPEG, TGA, BMP, PSD, GIF, HDR, PNM and the
  in-house DDS reader. Headers declare sizes, offsets, bit depths, masks
  and formats that can contradict each other or the real file length.
- BCn block decoding and encoding, including the third-party Compressonator
  CMP_Core code we call through its C API.
- Images that are valid but extreme: huge declared dimensions, very large
  pixel counts (decompression bombs), 1xN strips, odd sizes that are not
  multiples of the 4x4 block size.
- File and directory names from `-r` scans that end up in `{name}`,
  `{rel}` and `{dir}` output placeholders, plus symlinks and special files
  inside the scanned trees.
- Command line values (geometry, colors, numbers, op arguments), because
  wrapper scripts and services often forward them from untrusted sources.

### Rules for code changes

- Validate every header field against the actual buffer length before
  using it. Compute offsets and sizes with checked or widened arithmetic so
  no multiplication or addition can overflow. Never trust a declared size,
  mip count or pitch.
- Enforce the dimension limit (`MAX_DIMENSION`, 65536 per side) and the
  pixel count limit (`MAX_PIXELS`) through `check_dimensions()` in
  `src/img/image.rs` before allocating, both after decoding and for every
  operation that produces a new size (`--resize`, `--canvas`, `--pad`,
  `--tile`, `--rotate expand=1`, ...).
- No panics reachable from input data: no indexing or slicing that is not
  bounded by a validated size, no `unwrap()`/`expect()` on anything
  derived from input, no out-of-range shifts, no `as` casts that silently
  truncate or wrap an unchecked value. Turn every failure into an ordinary
  `Error` carrying the input name. Each run is wrapped in `catch_unwind`
  so that a panic only fails its own input, which is why the release
  profile must keep `panic = "unwind"`; a panic is still a bug.
- `./dev.py test` runs with debug assertions and integer overflow checks,
  so arithmetic that overflows on crafted input fails the tests instead of
  wrapping silently as in release builds. Prefer `checked_*` arithmetic or
  `u64` intermediates in the code itself.
- Keep loops bounded by validated sizes, so that crafted data cannot cause
  an endless loop or quadratic blowup.
- Third-party code (the `image` and `png` crates, CMP_Core) is part of the
  attack surface. Keep exact versions in `Cargo.lock` (always build with
  `--locked`) and enable only the decoder features that are needed. CMP_Core
  is C++ behind the FFI in `src/img/bcn.rs` and `cmp/cmp_capi.cpp`, so none
  of Rust's guarantees hold there: keep the local patches in `build.rs` that
  remove its undefined behaviour, and only hand it complete 4x4 blocks and
  buffers whose sizes were checked on the Rust side. Every `unsafe` block
  carries a `// SAFETY:` comment stating why its pointers and sizes are
  valid. When updating dependencies, look at their security fixes.
- Placeholders expanded from input paths must not be able to escape the
  output root the user asked for. Paths derived from scanned files are
  relative to the scanned directory. Keep it that way.
- The directory scan (`list_files()` in `src/common/fs.rs`) follows
  symlinks, including ones pointing outside the tree, but visits every
  directory (by canonical path) only once, so symlink loops terminate. It
  only collects regular files: FIFOs, devices, sockets, dangling links and
  file symlink loops are skipped, since opening a FIFO blocks forever.
  `read_file()` refuses non-regular files given directly as inputs for the
  same reason. Keep both properties when touching this code.
- Add a regression test for every bug found with a crafted file
  (truncated headers, contradictory sizes, zero or maximum dimensions,
  bogus masks, bad FourCC/DXGI values). Decoder cases go into
  `tests/hostile.rs`, cases that need the whole executable (exit codes,
  scanned trees, special files) into `tests/cli.rs`, whose runs are bounded
  by a timeout so that hangs fail the test. Fuzzing the decoders and the
  `img` pipeline is encouraged, and any finding gets a test as well.
