//! Block compression (BCn / S3TC) on top of Compressonator's CMP_Core,
//! reached through the C wrappers in cmp/cmp_capi.cpp.
//!
//! Only complete 4x4 blocks and correctly sized buffers are handed to
//! CMP_Core: partial edge blocks are padded here, and every slice passed
//! across the FFI boundary is a fixed-size array.

use std::ffi::{c_int, c_uint, c_void};

use rayon::prelude::*;

use crate::common::{Error, Result};

unsafe extern "C" {
    fn idt3tc_cmp_create_options(bc: c_int, options: *mut *mut c_void) -> c_int;
    fn idt3tc_cmp_destroy_options(bc: c_int, options: *mut c_void) -> c_int;
    fn idt3tc_cmp_set_quality(bc: c_int, options: *mut c_void, quality: f32) -> c_int;
    fn idt3tc_cmp_set_channel_weights(bc: c_int, options: *mut c_void, r: f32, g: f32, b: f32) -> c_int;
    fn idt3tc_cmp_set_refine_steps_bc1(options: *mut c_void, steps: c_uint) -> c_int;
    fn idt3tc_cmp_compress_rgba(bc: c_int, rgba: *const u8, out: *mut u8, options: *const c_void) -> c_int;
    fn idt3tc_cmp_compress_bc4(r: *const u8, out: *mut u8, options: *const c_void) -> c_int;
    fn idt3tc_cmp_compress_bc5(r: *const u8, g: *const u8, out: *mut u8, options: *const c_void) -> c_int;
    fn idt3tc_cmp_decompress_rgba(bc: c_int, input: *const u8, rgba: *mut u8) -> c_int;
    fn idt3tc_cmp_decompress_bc4(input: *const u8, r: *mut u8) -> c_int;
    fn idt3tc_cmp_decompress_bc5(input: *const u8, r: *mut u8, g: *mut u8) -> c_int;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFormat {
    /// DXT1, opaque RGB
    Bc1,
    /// DXT1 with 1-bit (punch-through) alpha
    Bc1a,
    /// DXT3, explicit 4-bit alpha
    Bc2,
    /// DXT5, interpolated alpha
    Bc3,
    /// ATI1, single channel (red)
    Bc4,
    /// ATI2, two channels (red, green), e.g. normal maps
    Bc5,
    /// High quality RGBA, requires the DX10 DDS header
    Bc7,
}

impl BlockFormat {
    /// The BCn number used to select the CMP_Core codec.
    fn bc(self) -> c_int {
        match self {
            BlockFormat::Bc1 | BlockFormat::Bc1a => 1,
            BlockFormat::Bc2 => 2,
            BlockFormat::Bc3 => 3,
            BlockFormat::Bc4 => 4,
            BlockFormat::Bc5 => 5,
            BlockFormat::Bc7 => 7,
        }
    }
}

/// Bytes per 4x4 block.
pub fn block_bytes(f: BlockFormat) -> usize {
    match f {
        BlockFormat::Bc1 | BlockFormat::Bc1a | BlockFormat::Bc4 => 8,
        _ => 16,
    }
}

/// Size of the compressed data for one w x h surface. Dimensions are
/// limited to 65536, so this cannot overflow on 64-bit targets; checked
/// arithmetic keeps it safe everywhere else as well.
pub fn compressed_size(f: BlockFormat, w: i32, h: i32) -> Option<usize> {
    let bw = (w.max(0) as usize).div_ceil(4);
    let bh = (h.max(0) as usize).div_ceil(4);
    bw.checked_mul(bh)?.checked_mul(block_bytes(f))
}

#[derive(Debug, Clone, Copy)]
pub struct BlockOptions {
    /// 0..1, Compressonator's speed/quality trade-off
    pub quality: f32,
    /// BC1/BC1A endpoint refinement iterations (0..)
    pub refine_steps: i32,
    /// Use `weights` for the color error metric (BC1-3)
    pub use_weights: bool,
    /// Perceptual channel weights (r, g, b)
    pub weights: [f32; 3],
    /// BC1A: alpha below this becomes transparent
    pub alpha_threshold: i32,
}

impl Default for BlockOptions {
    fn default() -> Self {
        BlockOptions {
            quality: 1.0,
            refine_steps: 0,
            use_weights: false,
            weights: [0.3086, 0.6094, 0.0820],
            alpha_threshold: 128,
        }
    }
}

fn check(rc: c_int, what: &str) -> Result<()> {
    if rc != 0 {
        return Err(Error::runtime(format!("Compressonator: {what} failed (error {rc})")));
    }
    Ok(())
}

/// Owns a CMP_Core option context.
struct CmpOptions {
    bc: c_int,
    ptr: *mut c_void,
}

// The context is only read by the compress calls, and CMP_Core keeps no
// mutable global state in the encode paths, so sharing it between the
// worker threads is sound.
unsafe impl Send for CmpOptions {}
unsafe impl Sync for CmpOptions {}

impl Drop for CmpOptions {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr was created by idt3tc_cmp_create_options for the
            // same codec and is destroyed exactly once.
            unsafe { idt3tc_cmp_destroy_options(self.bc, self.ptr) };
        }
    }
}

impl CmpOptions {
    fn new(f: BlockFormat, opts: &BlockOptions) -> Result<Self> {
        let bc = f.bc();
        let mut o = CmpOptions { bc, ptr: std::ptr::null_mut() };
        let q = opts.quality.clamp(0.0, 1.0);
        let n = bc;
        // SAFETY: the out pointer is valid; every following call receives
        // the context just created (checked for success first).
        unsafe {
            check(idt3tc_cmp_create_options(bc, &mut o.ptr), &format!("CreateOptionsBC{n}"))?;
            if o.ptr.is_null() {
                return Err(Error::runtime(format!("Compressonator: CreateOptionsBC{n} failed (no context)")));
            }
            check(idt3tc_cmp_set_quality(bc, o.ptr, q), &format!("SetQualityBC{n}"))?;
            if bc == 1 {
                let steps = opts.refine_steps.max(0) as c_uint;
                check(idt3tc_cmp_set_refine_steps_bc1(o.ptr, steps), "SetRefineStepsBC1")?;
            }
            if opts.use_weights && bc <= 3 {
                let [r, g, b] = opts.weights;
                check(idt3tc_cmp_set_channel_weights(bc, o.ptr, r, g, b), &format!("SetChannelWeightsBC{n}"))?;
            }
        }
        Ok(o)
    }
}

// --- BC1 helpers ------------------------------------------------------------
//
// CMP_Core's BC1 encoder ignores the alpha threshold, and may pick the
// 3-color mode (color0 <= color1) whose 4th index decodes as transparent
// black on most hardware. For idTech 3 this matters: opaque DXT1 must never
// contain such texels, and DXT1a needs real punch-through alpha. Both are
// handled here by re-indexing against CMP_Core's endpoints.

fn decode565(c: u32) -> [i32; 3] {
    let r = ((c >> 11) & 31) as i32;
    let g = ((c >> 5) & 63) as i32;
    let b = (c & 31) as i32;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

fn color_distance(a: &[i32; 3], b: &[u8], w: &[f32; 3]) -> i32 {
    let mut d = 0.0f32;
    for c in 0..3 {
        let diff = (a[c] - b[c] as i32) as f32;
        d += diff * diff * w[c];
    }
    (d * 16.0) as i32
}

fn write_bc1(out: &mut [u8], c0: u32, c1: u32, indices: u32) {
    out[0..2].copy_from_slice(&(c0 as u16).to_le_bytes());
    out[2..4].copy_from_slice(&(c1 as u16).to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
}

/// Re-indexes `block` (16 RGBA texels) against endpoints c0/c1.
/// four_color: c0 > c1 is required by the caller; otherwise 3-color mode
/// with index 3 used for texels flagged in `transparent`.
fn reindex(block: &[u8; 64], c0: u32, c1: u32, four_color: bool, transparent: &[bool; 16], w: &[f32; 3]) -> u32 {
    let mut p = [[0i32; 3]; 4];
    p[0] = decode565(c0);
    p[1] = decode565(c1);
    let num_colors = if four_color { 4 } else { 3 };
    for c in 0..3 {
        if four_color {
            p[2][c] = (2 * p[0][c] + p[1][c]) / 3;
            p[3][c] = (p[0][c] + 2 * p[1][c]) / 3;
        } else {
            p[2][c] = (p[0][c] + p[1][c]) / 2;
        }
    }
    let mut indices = 0u32;
    for i in 0..16 {
        let texel = &block[i * 4..i * 4 + 4];
        let idx = if !four_color && transparent[i] {
            3
        } else {
            let mut best = 0;
            let mut best_dist = color_distance(&p[0], texel, w);
            for (k, pk) in p.iter().enumerate().take(num_colors).skip(1) {
                let d = color_distance(pk, texel, w);
                if d < best_dist {
                    best_dist = d;
                    best = k as u32;
                }
            }
            best
        };
        indices |= idx << (2 * i);
    }
    indices
}

fn compress_bc1(block: &[u8; 64], out: &mut [u8], punch_through: bool, opts: &BlockOptions, cmp: &CmpOptions) -> Result<()> {
    let w = if opts.use_weights { opts.weights } else { [1.0; 3] };
    let mut transparent = [false; 16];
    let mut num_transparent = 0;
    for (i, t) in transparent.iter_mut().enumerate() {
        *t = punch_through && (block[i * 4 + 3] as i32) < opts.alpha_threshold;
        num_transparent += *t as i32;
    }
    if num_transparent == 16 {
        // Fully transparent: 3-color mode, every index 3.
        write_bc1(out, 0, 0, 0xffff_ffff);
        return Ok(());
    }
    // Transparent texels must not influence the endpoints: replace them by
    // the average opaque color before running the encoder.
    let mut work = *block;
    if num_transparent > 0 {
        let mut sum = [0i32; 3];
        for i in (0..16).filter(|&i| !transparent[i]) {
            for c in 0..3 {
                sum[c] += block[i * 4 + c] as i32;
            }
        }
        let opaque_count = 16 - num_transparent;
        for i in (0..16).filter(|&i| transparent[i]) {
            for c in 0..3 {
                work[i * 4 + c] = (sum[c] / opaque_count) as u8;
            }
        }
    }
    for i in 0..16 {
        work[i * 4 + 3] = 255;
    }
    let mut enc = [0u8; 8];
    // SAFETY: work is a full 64 byte block, enc holds the 8 output bytes.
    check(unsafe { idt3tc_cmp_compress_rgba(1, work.as_ptr(), enc.as_mut_ptr(), cmp.ptr) }, "CompressBlockBC1")?;
    let c0 = u16::from_le_bytes([enc[0], enc[1]]) as u32;
    let c1 = u16::from_le_bytes([enc[2], enc[3]]) as u32;
    if num_transparent > 0 {
        // 3-color mode requires c0 <= c1.
        let (lo, hi) = (c0.min(c1), c0.max(c1));
        write_bc1(out, lo, hi, reindex(&work, lo, hi, false, &transparent, &w));
        return Ok(());
    }
    if c0 > c1 {
        // Already 4-color mode, keep the encoder result.
        out[..8].copy_from_slice(&enc);
        return Ok(());
    }
    // Convert to 4-color mode: order endpoints descending; equal endpoints
    // get nudged apart by one LSB of blue so that c0 > c1 holds.
    let (mut hi, mut lo) = (c0.max(c1), c0.min(c1));
    if hi == lo {
        if lo > 0 {
            lo -= 1;
        } else {
            hi += 1;
        }
    }
    write_bc1(out, hi, lo, reindex(&work, hi, lo, true, &[false; 16], &w));
    Ok(())
}

/// Fills a 4x4 RGBA block from the surface, replicating edge texels.
fn gather_block(rgba: &[u8], w: usize, h: usize, bx: usize, by: usize) -> [u8; 64] {
    let mut block = [0u8; 64];
    for y in 0..4 {
        let sy = (by * 4 + y).min(h - 1);
        for x in 0..4 {
            let sx = (bx * 4 + x).min(w - 1);
            let s = (sy * w + sx) * 4;
            block[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    block
}

fn compress_block(f: BlockFormat, block: &[u8; 64], dst: &mut [u8], opts: &BlockOptions, cmp: &CmpOptions) -> Result<()> {
    let ptr = cmp.ptr as *const c_void;
    match f {
        BlockFormat::Bc1 => compress_bc1(block, dst, false, opts, cmp),
        BlockFormat::Bc1a => compress_bc1(block, dst, true, opts, cmp),
        BlockFormat::Bc2 | BlockFormat::Bc3 | BlockFormat::Bc7 => {
            debug_assert_eq!(dst.len(), 16);
            let what = match f {
                BlockFormat::Bc2 => "CompressBlockBC2",
                BlockFormat::Bc3 => "CompressBlockBC3",
                _ => "CompressBlockBC7",
            };
            // SAFETY: full 64 byte source block, 16 byte destination.
            check(unsafe { idt3tc_cmp_compress_rgba(f.bc(), block.as_ptr(), dst.as_mut_ptr(), ptr) }, what)
        }
        BlockFormat::Bc4 => {
            debug_assert_eq!(dst.len(), 8);
            let r: [u8; 16] = std::array::from_fn(|i| block[i * 4]);
            // SAFETY: 16 byte source channel, 8 byte destination.
            check(unsafe { idt3tc_cmp_compress_bc4(r.as_ptr(), dst.as_mut_ptr(), ptr) }, "CompressBlockBC4")
        }
        BlockFormat::Bc5 => {
            debug_assert_eq!(dst.len(), 16);
            let r: [u8; 16] = std::array::from_fn(|i| block[i * 4]);
            let g: [u8; 16] = std::array::from_fn(|i| block[i * 4 + 1]);
            // SAFETY: two 16 byte source channels, 16 byte destination.
            check(
                unsafe { idt3tc_cmp_compress_bc5(r.as_ptr(), g.as_ptr(), dst.as_mut_ptr(), ptr) },
                "CompressBlockBC5",
            )
        }
    }
}

/// Compresses a tightly packed RGBA8 surface. Partial edge blocks are
/// padded by replicating the last row/column. Block rows are compressed in
/// parallel on the current rayon pool.
pub fn compress_surface(rgba: &[u8], w: i32, h: i32, f: BlockFormat, opts: &BlockOptions) -> Result<Vec<u8>> {
    if w <= 0 || h <= 0 || rgba.len() != w as usize * h as usize * 4 {
        return Err(Error::runtime("compress_surface: invalid surface size"));
    }
    let (wu, hu) = (w as usize, h as usize);
    let bw = wu.div_ceil(4);
    let bsize = block_bytes(f);
    let size = compressed_size(f, w, h).ok_or_else(|| Error::runtime("compressed surface too large"))?;
    let mut out = vec![0u8; size];
    let cmp = CmpOptions::new(f, opts)?;
    out.par_chunks_mut(bw * bsize).enumerate().try_for_each(|(by, row)| -> Result<()> {
        for (bx, dst) in row.chunks_exact_mut(bsize).enumerate() {
            let block = gather_block(rgba, wu, hu, bx, by);
            compress_block(f, &block, dst, opts, &cmp)?;
        }
        Ok(())
    })?;
    Ok(out)
}

/// BC1 decoding done locally so 3-color mode is decoded as on hardware
/// (index 3 = transparent black), independent of CMP_Core's decoder flavor.
fn decode_bc1_block(src: &[u8], force_four_color: bool) -> [u8; 64] {
    let c0 = u16::from_le_bytes([src[0], src[1]]) as u32;
    let c1 = u16::from_le_bytes([src[2], src[3]]) as u32;
    let mut p = [[0i32; 4]; 4];
    let (e0, e1) = (decode565(c0), decode565(c1));
    p[0][..3].copy_from_slice(&e0);
    p[1][..3].copy_from_slice(&e1);
    p[0][3] = 255;
    p[1][3] = 255;
    let four = force_four_color || c0 > c1;
    for c in 0..3 {
        if four {
            p[2][c] = (2 * p[0][c] + p[1][c]) / 3;
            p[3][c] = (p[0][c] + 2 * p[1][c]) / 3;
        } else {
            p[2][c] = (p[0][c] + p[1][c]) / 2;
            p[3][c] = 0;
        }
    }
    p[2][3] = 255;
    p[3][3] = if four { 255 } else { 0 };
    let idx = u32::from_le_bytes([src[4], src[5], src[6], src[7]]);
    let mut out = [0u8; 64];
    for i in 0..16 {
        let k = ((idx >> (2 * i)) & 3) as usize;
        for c in 0..4 {
            out[i * 4 + c] = p[k][c] as u8;
        }
    }
    out
}

fn decode_block(f: BlockFormat, src: &[u8]) -> Result<[u8; 64]> {
    let mut block = [0u8; 64];
    match f {
        BlockFormat::Bc1 | BlockFormat::Bc1a => block = decode_bc1_block(src, false),
        BlockFormat::Bc2 | BlockFormat::Bc3 => {
            // The color part of BC2/BC3 is always 4-color mode.
            block = decode_bc1_block(&src[8..16], true);
            let mut tmp = [0u8; 64];
            let what = if f == BlockFormat::Bc2 { "DecompressBlockBC2" } else { "DecompressBlockBC3" };
            // SAFETY: src holds 16 bytes (block_bytes), tmp 64.
            check(unsafe { idt3tc_cmp_decompress_rgba(f.bc(), src.as_ptr(), tmp.as_mut_ptr()) }, what)?;
            for i in 0..16 {
                block[i * 4 + 3] = tmp[i * 4 + 3];
            }
        }
        BlockFormat::Bc4 => {
            let mut r = [0u8; 16];
            // SAFETY: src holds 8 bytes, r 16.
            check(unsafe { idt3tc_cmp_decompress_bc4(src.as_ptr(), r.as_mut_ptr()) }, "DecompressBlockBC4")?;
            for i in 0..16 {
                block[i * 4..i * 4 + 4].copy_from_slice(&[r[i], r[i], r[i], 255]);
            }
        }
        BlockFormat::Bc5 => {
            let (mut r, mut g) = ([0u8; 16], [0u8; 16]);
            // SAFETY: src holds 16 bytes, r and g 16 each.
            check(
                unsafe { idt3tc_cmp_decompress_bc5(src.as_ptr(), r.as_mut_ptr(), g.as_mut_ptr()) },
                "DecompressBlockBC5",
            )?;
            for i in 0..16 {
                block[i * 4..i * 4 + 4].copy_from_slice(&[r[i], g[i], 0, 255]);
            }
        }
        BlockFormat::Bc7 => {
            // SAFETY: src holds 16 bytes, block 64.
            check(unsafe { idt3tc_cmp_decompress_rgba(7, src.as_ptr(), block.as_mut_ptr()) }, "DecompressBlockBC7")?;
        }
    }
    Ok(block)
}

/// Decompresses one surface to tightly packed RGBA8. BC4 is expanded to
/// gray, BC5 to (r, g, 0).
pub fn decompress_surface(data: &[u8], w: i32, h: i32, f: BlockFormat) -> Result<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return Err(Error::runtime("compressed surface has an invalid size"));
    }
    let size = compressed_size(f, w, h).ok_or_else(|| Error::runtime("compressed surface too large"))?;
    if data.len() < size {
        return Err(Error::runtime("compressed surface is truncated"));
    }
    let (wu, hu) = (w as usize, h as usize);
    let bw = wu.div_ceil(4);
    let bsize = block_bytes(f);
    let mut out = vec![0u8; wu * hu * 4];
    // One chunk per row of blocks (4 pixel rows, fewer at the bottom edge).
    out.par_chunks_mut(wu * 4 * 4).enumerate().try_for_each(|(by, rows)| -> Result<()> {
        for bx in 0..bw {
            let off = (by * bw + bx) * bsize;
            let block = decode_block(f, &data[off..off + bsize])?;
            for (y, row) in rows.chunks_exact_mut(wu * 4).enumerate() {
                let x0 = bx * 4;
                let n = 4.min(wu - x0);
                row[x0 * 4..(x0 + n) * 4].copy_from_slice(&block[y * 16..y * 16 + n * 4]);
            }
        }
        Ok(())
    })?;
    Ok(out)
}
