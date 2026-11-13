//! TGA encoder. Decoding goes through the generic image loader.

use super::image::{Image, analyze_alpha, luma, to_byte};
use crate::common::{Error, Result};

/// Defaults match what the idTech 3 TGA loader handles: uncompressed,
/// bottom-up rows (ioquake3 ignores the top-down flag and would show the
/// texture upside down).
#[derive(Debug, Clone, Copy, Default)]
pub struct TgaOptions {
    /// 0 = auto (32 with alpha, else 24), 8 (gray), 24, 32.
    pub bpp: i32,
    /// RLE compression (image types 10 and 11).
    pub rle: bool,
    /// Rows stored top-down (descriptor bit 5).
    pub top_origin: bool,
}

fn put16(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&(v as u16).to_le_bytes());
}

/// Appends one scanline as RLE packets. Packets never cross scanlines, as
/// recommended by the TGA 2.0 spec (some loaders cannot handle it).
fn rle_scanline(row: &[u8], px: usize, out: &mut Vec<u8>) {
    let width = row.len() / px;
    let pixel = |i: usize| &row[i * px..(i + 1) * px];
    let mut i = 0;
    while i < width {
        // Length of the run of identical pixels starting at i.
        let mut run = 1;
        while i + run < width && run < 128 && pixel(i) == pixel(i + run) {
            run += 1;
        }
        if run >= 2 {
            out.push(0x80 | (run - 1) as u8);
            out.extend_from_slice(pixel(i));
            i += run;
            continue;
        }
        // Raw packet: extend until a repeat starts or 128 pixels.
        let start = i;
        let mut count = 0;
        while i < width && count < 128 {
            if i + 1 < width && pixel(i) == pixel(i + 1) {
                break;
            }
            i += 1;
            count += 1;
        }
        out.push((count - 1) as u8);
        out.extend_from_slice(&row[start * px..(start + count) * px]);
    }
}

/// Returns the file bytes and the resolved bit depth.
pub fn encode_tga(img: &Image, opts: &TgaOptions) -> Result<(Vec<u8>, i32)> {
    if img.is_empty() {
        return Err(Error::runtime("TGA: empty image"));
    }
    if img.width > 65535 || img.height > 65535 {
        return Err(Error::runtime("TGA: image too large (max 65535x65535)"));
    }
    let mut bpp = opts.bpp;
    if bpp == 0 {
        bpp = if analyze_alpha(img).opaque { 24 } else { 32 };
    }
    if bpp != 8 && bpp != 24 && bpp != 32 {
        return Err(Error::usage(format!("TGA: unsupported bit depth {bpp} (use 8, 24 or 32)")));
    }
    let gray = bpp == 8;
    let px = (bpp / 8) as usize;
    let mut out = Vec::with_capacity(18 + img.pixel_count() * px);
    out.push(0); // no image ID
    out.push(0); // no color map
    // 2 = truecolor, 3 = grayscale, +8 = RLE
    out.push((if gray { 3 } else { 2 }) + if opts.rle { 8 } else { 0 });
    out.extend_from_slice(&[0; 5]); // color map spec
    put16(&mut out, 0); // x origin
    put16(&mut out, 0); // y origin
    put16(&mut out, img.width);
    put16(&mut out, img.height);
    out.push(bpp as u8);
    // Descriptor: alpha bits in the low nibble, bit 5 = top-down rows.
    out.push((if bpp == 32 { 8 } else { 0 }) | if opts.top_origin { 0x20 } else { 0 });

    let mut row = vec![0u8; img.width as usize * px];
    for i in 0..img.height {
        let y = if opts.top_origin { i } else { img.height - 1 - i };
        for (p, d) in img.row(y).chunks_exact(4).zip(row.chunks_exact_mut(px)) {
            if gray {
                d[0] = to_byte(luma(p));
            } else {
                // TGA stores BGR(A).
                d[0] = to_byte(p[2]);
                d[1] = to_byte(p[1]);
                d[2] = to_byte(p[0]);
                if px == 4 {
                    d[3] = to_byte(p[3]);
                }
            }
        }
        if opts.rle {
            rle_scanline(&row, px, &mut out);
        } else {
            out.extend_from_slice(&row);
        }
    }
    Ok((out, bpp))
}
