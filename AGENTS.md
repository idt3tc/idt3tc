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
- Enforce the dimension limit (`kMaxDimension`, 65536 per side) and check
  the total pixel count and allocation size before allocating, both after
  decoding and for every operation that produces a new size (`--resize`,
  `--canvas`, `--pad`, `--tile`, `--rotate expand=1`, ...).
- No unchecked indexing, no out-of-range shifts, no signed overflow, no
  `abort()` or panics reachable from input data. Turn every failure into an
  ordinary error carrying the input name.
- Keep loops bounded by validated sizes, so that crafted data cannot cause
  an endless loop or quadratic blowup.
- Third-party code (decoders, CMP_Core) is part of the attack surface. Pin
  exact versions with checksums and keep the local patches that remove
  undefined behaviour. Only hand complete, valid 4x4 blocks and correctly
  sized buffers to CMP_Core. When updating such dependencies, look at
  their security fixes, and prefer memory-safe implementations where
  available.
- Placeholders expanded from input paths must not be able to escape the
  output root the user asked for. Paths derived from scanned files are
  relative to the scanned directory. Keep it that way. Note that the
  directory scan currently uses `stat()` and therefore follows symlinks,
  including ones pointing outside the tree and symlink loops. Keep that in
  mind when touching the scan code.
- Add a regression test for every bug found with a crafted file
  (truncated headers, contradictory sizes, zero or maximum dimensions,
  bogus masks, bad FourCC/DXGI values). Fuzzing the decoders and the
  `img` pipeline is encouraged, and any finding gets a test as well.
