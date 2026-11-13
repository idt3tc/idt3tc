//! DDS encoder (block compressed and mask based uncompressed formats, with
//! mipmaps) and a decoder for the top mip level.
//!
//! The decoder sees untrusted files: every header field is validated
//! against the buffer length, and only sizes derived from validated
//! dimensions are used.

use super::bcn::{BlockFormat, BlockOptions, compress_surface, compressed_size, decompress_surface};
use super::image::{Image, analyze_alpha, check_dimensions, from_rgba8, luma, to_byte, to_rgba8, unpremultiply};
use super::mipmap::{MipOptions, build_mip_chain};
use crate::common::{Error, Result};

// DDS_PIXELFORMAT flags
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x20000;

// DDS_HEADER flags
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PITCH: u32 = 0x8;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x20000;
const DDSD_LINEARSIZE: u32 = 0x80000;

const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;

const DDS_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;

const fn four_cc(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsCodec {
    Auto,
    Bc1,
    Bc1a,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
    Rgba8,
    Bgra8,
    Bgrx8,
    Bgr8,
    L8,
    A8,
}

/// Static properties of each output codec.
struct CodecDef {
    codec: DdsCodec,
    name: &'static str,
    /// Legacy name, may be empty.
    alias: &'static str,
    /// Block compressed format, None for mask formats.
    block: Option<BlockFormat>,
    /// Legacy header FourCC, "" for mask formats.
    fourcc: &'static str,
    /// DXGI_FORMAT, 0 = none.
    dxgi: u32,
    /// sRGB variant, 0 = none.
    dxgi_srgb: u32,
    /// Mask formats only.
    pf_flags: u32,
    bits: u32,
    /// r, g, b, a
    masks: [u32; 4],
}

const fn block_def(
    codec: DdsCodec,
    name: &'static str,
    alias: &'static str,
    bf: BlockFormat,
    fourcc: &'static str,
    dxgi: u32,
    dxgi_srgb: u32,
) -> CodecDef {
    CodecDef { codec, name, alias, block: Some(bf), fourcc, dxgi, dxgi_srgb, pf_flags: 0, bits: 0, masks: [0; 4] }
}

const fn mask_def(
    codec: DdsCodec,
    name: &'static str,
    alias: &'static str,
    dxgi: u32,
    dxgi_srgb: u32,
    pf_flags: u32,
    bits: u32,
    masks: [u32; 4],
) -> CodecDef {
    CodecDef { codec, name, alias, block: None, fourcc: "", dxgi, dxgi_srgb, pf_flags, bits, masks }
}

const CODECS: &[CodecDef] = &[
    block_def(DdsCodec::Bc1, "bc1", "dxt1", BlockFormat::Bc1, "DXT1", 71, 72),
    block_def(DdsCodec::Bc1a, "bc1a", "dxt1a", BlockFormat::Bc1a, "DXT1", 71, 72),
    block_def(DdsCodec::Bc2, "bc2", "dxt3", BlockFormat::Bc2, "DXT3", 74, 75),
    block_def(DdsCodec::Bc3, "bc3", "dxt5", BlockFormat::Bc3, "DXT5", 77, 78),
    block_def(DdsCodec::Bc4, "bc4", "ati1", BlockFormat::Bc4, "ATI1", 80, 0),
    block_def(DdsCodec::Bc5, "bc5", "ati2", BlockFormat::Bc5, "ATI2", 83, 0),
    block_def(DdsCodec::Bc7, "bc7", "", BlockFormat::Bc7, "", 98, 99),
    mask_def(DdsCodec::Rgba8, "rgba8", "rgba", 28, 29, DDPF_RGB | DDPF_ALPHAPIXELS, 32, [
        0xff,
        0xff00,
        0xff_0000,
        0xff00_0000,
    ]),
    mask_def(DdsCodec::Bgra8, "bgra8", "bgra", 87, 91, DDPF_RGB | DDPF_ALPHAPIXELS, 32, [
        0xff_0000,
        0xff00,
        0xff,
        0xff00_0000,
    ]),
    mask_def(DdsCodec::Bgrx8, "bgrx8", "bgrx", 88, 93, DDPF_RGB, 32, [0xff_0000, 0xff00, 0xff, 0]),
    mask_def(DdsCodec::Bgr8, "bgr8", "bgr", 0, 0, DDPF_RGB, 24, [0xff_0000, 0xff00, 0xff, 0]),
    mask_def(DdsCodec::L8, "l8", "luminance", 61, 0, DDPF_LUMINANCE, 8, [0xff, 0, 0, 0]),
    mask_def(DdsCodec::A8, "a8", "alpha", 65, 0, DDPF_ALPHA, 8, [0, 0, 0, 0xff]),
];

fn codec_def(c: DdsCodec) -> Result<&'static CodecDef> {
    CODECS.iter().find(|d| d.codec == c).ok_or_else(|| Error::runtime("DDS: unresolved codec"))
}

pub fn parse_dds_codec(s_in: &str) -> Option<DdsCodec> {
    let mut s = s_in.to_ascii_lowercase();
    if s == "auto" {
        return Some(DdsCodec::Auto);
    }
    // No special handling for these, same containers.
    match s.as_str() {
        "dxt5n" => s = "bc3".into(),
        "bc4u" => s = "bc4".into(),
        "bc5u" => s = "bc5".into(),
        _ => {}
    }
    CODECS.iter().find(|d| s == d.name || (!d.alias.is_empty() && s == d.alias)).map(|d| d.codec)
}

pub fn dds_codec_name(c: DdsCodec) -> String {
    match CODECS.iter().find(|d| d.codec == c) {
        None => "auto".to_string(),
        Some(d) if d.alias.is_empty() => d.name.to_string(),
        Some(d) => format!("{}/{}", d.name, d.alias),
    }
}

pub fn dds_codec_names() -> Vec<String> {
    std::iter::once("auto".to_string()).chain(CODECS.iter().map(|d| dds_codec_name(d.codec))).collect()
}

#[derive(Debug, Clone, Copy)]
pub struct DdsOptions {
    pub codec: DdsCodec,
    /// mips.levels == 1 disables mipmaps
    pub mips: MipOptions,
    pub block: BlockOptions,
    /// -1 auto (only when required), 0 never, 1 always
    pub dx10: i32,
    /// Tag as sRGB (DX10 header only).
    pub srgb: bool,
}

impl Default for DdsOptions {
    fn default() -> Self {
        DdsOptions {
            codec: DdsCodec::Auto,
            mips: MipOptions::default(),
            block: BlockOptions::default(),
            dx10: -1,
            srgb: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DdsResult {
    pub bytes: Vec<u8>,
    /// Resolved codec.
    pub codec: DdsCodec,
    pub mip_levels: i32,
    pub dx10: bool,
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Reads a little endian u32 at `off`; callers check the length first.
fn get32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Packs a quantized RGBA8 surface into a mask based layout.
fn pack_uncompressed(def: &CodecDef, level: &Image, out: &mut Vec<u8>) {
    let bytes = (def.bits / 8) as usize;
    for p in level.data.chunks_exact(4) {
        let rgba = [to_byte(p[0]), to_byte(p[1]), to_byte(p[2]), to_byte(p[3])];
        let mut v: u32 = 0;
        if def.codec == DdsCodec::L8 {
            v = to_byte(luma(p)) as u32;
        } else {
            for (c, &m) in def.masks.iter().enumerate() {
                if m != 0 {
                    v |= (rgba[c] as u32) << m.trailing_zeros();
                }
            }
        }
        out.extend_from_slice(&v.to_le_bytes()[..bytes]);
    }
}

/// Extracts a channel described by `mask` and expands it to 8 bits. Masks
/// come from untrusted headers: only the lowest contiguous run of set bits
/// is used, so gaps or stray bits cannot produce values above 255.
fn extract_masked(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).trailing_ones();
    let maxv: u64 = if bits >= 32 { u32::MAX as u64 } else { (1u64 << bits) - 1 };
    let x = ((v >> shift) as u64) & maxv;
    ((x * 255 + maxv / 2) / maxv) as u8
}

pub fn encode_dds(img: &Image, opts: &DdsOptions) -> Result<DdsResult> {
    if img.is_empty() {
        return Err(Error::runtime("DDS: empty image"));
    }
    let codec =
        if opts.codec == DdsCodec::Auto { if analyze_alpha(img).opaque { DdsCodec::Bc1 } else { DdsCodec::Bc3 } } else { opts.codec };
    let def = codec_def(codec)?;

    let dx10 = opts.dx10 == 1 || (opts.dx10 == -1 && (codec == DdsCodec::Bc7 || opts.srgb));
    if dx10 && def.dxgi == 0 {
        return Err(Error::usage(format!("DDS: codec {} has no DX10/DXGI equivalent (use dx10=0)", def.name)));
    }
    if !dx10 && def.fourcc.is_empty() && def.pf_flags == 0 {
        return Err(Error::usage(format!("DDS: codec {} requires the DX10 header (dx10=1)", def.name)));
    }
    if opts.srgb && !dx10 {
        return Err(Error::usage("DDS: srgb=1 requires the DX10 header"));
    }
    if opts.srgb && def.dxgi_srgb == 0 {
        return Err(Error::usage(format!("DDS: codec {} has no sRGB variant", def.name)));
    }

    let chain = build_mip_chain(img, &opts.mips)?;
    let mipped = chain.len() > 1;

    // --- header ---
    let mut out = Vec::new();
    out.extend_from_slice(b"DDS ");
    let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
    // Dimensions are at most 65536, so the top level size fits easily; the
    // field is 32 bits wide, saturate rather than wrap for safety.
    let pitch_or_size = match def.block {
        Some(bf) => {
            flags |= DDSD_LINEARSIZE;
            compressed_size(bf, img.width, img.height).map_or(u32::MAX, |s| u32::try_from(s).unwrap_or(u32::MAX))
        }
        None => {
            flags |= DDSD_PITCH;
            ((img.width as u64 * def.bits as u64).div_ceil(8)) as u32
        }
    };
    if mipped {
        flags |= DDSD_MIPMAPCOUNT;
    }

    put32(&mut out, 124);
    put32(&mut out, flags);
    put32(&mut out, img.height as u32);
    put32(&mut out, img.width as u32);
    put32(&mut out, pitch_or_size);
    put32(&mut out, 0); // depth
    put32(&mut out, if mipped { chain.len() as u32 } else { 0 });
    for _ in 0..11 {
        put32(&mut out, 0); // reserved
    }

    // DDS_PIXELFORMAT
    put32(&mut out, 32);
    if dx10 {
        put32(&mut out, DDPF_FOURCC);
        put32(&mut out, four_cc(b"DX10"));
        for _ in 0..5 {
            put32(&mut out, 0);
        }
    } else if def.block.is_some() {
        put32(&mut out, DDPF_FOURCC | if codec == DdsCodec::Bc1a { DDPF_ALPHAPIXELS } else { 0 });
        let fcc: [u8; 4] = def.fourcc.as_bytes().try_into().map_err(|_| Error::runtime("DDS: bad FourCC"))?;
        put32(&mut out, four_cc(&fcc));
        for _ in 0..5 {
            put32(&mut out, 0);
        }
    } else {
        put32(&mut out, def.pf_flags);
        put32(&mut out, 0);
        put32(&mut out, def.bits);
        for m in def.masks {
            put32(&mut out, m);
        }
    }

    put32(&mut out, DDSCAPS_TEXTURE | if mipped { DDSCAPS_COMPLEX | DDSCAPS_MIPMAP } else { 0 });
    for _ in 0..4 {
        put32(&mut out, 0); // caps2..4, reserved2
    }

    if dx10 {
        put32(&mut out, if opts.srgb { def.dxgi_srgb } else { def.dxgi });
        put32(&mut out, DDS_RESOURCE_DIMENSION_TEXTURE2D);
        put32(&mut out, 0); // misc flags
        put32(&mut out, 1); // array size
        put32(&mut out, 0); // alpha mode unknown
    }

    // --- surfaces ---
    for level in &chain {
        match def.block {
            Some(bf) => {
                let rgba = to_rgba8(level);
                out.extend_from_slice(&compress_surface(&rgba, level.width, level.height, bf, &opts.block)?);
            }
            None => pack_uncompressed(def, level, &mut out),
        }
    }
    Ok(DdsResult { bytes: out, codec, mip_levels: chain.len() as i32, dx10 })
}

/// Pixel layout of the top level found in a header.
enum Layout {
    Block(BlockFormat),
    Masked { bits: u32, flags: u32, masks: [u32; 4] },
}

const RGBA_MASKS: [u32; 4] = [0xff, 0xff00, 0xff_0000, 0xff00_0000];
const BGRA_MASKS: [u32; 4] = [0xff_0000, 0xff00, 0xff, 0xff00_0000];

fn dxgi_layout(dxgi: u32, name: &str) -> Result<(Layout, String)> {
    use BlockFormat::*;
    let block = |bf, d: &str| Ok((Layout::Block(bf), d.to_string()));
    let masked = |bits, flags, masks, d: &str| Ok((Layout::Masked { bits, flags, masks }, d.to_string()));
    match dxgi {
        70..=72 => block(Bc1a, "BC1"),
        73..=75 => block(Bc2, "BC2"),
        76..=78 => block(Bc3, "BC3"),
        79 | 80 => block(Bc4, "BC4"),
        82 | 83 => block(Bc5, "BC5"),
        97..=99 => block(Bc7, "BC7"),
        27..=29 => masked(32, DDPF_RGB | DDPF_ALPHAPIXELS, RGBA_MASKS, "RGBA8"),
        87 | 90 | 91 => masked(32, DDPF_RGB | DDPF_ALPHAPIXELS, BGRA_MASKS, "BGRA8"),
        88 | 92 | 93 => masked(32, DDPF_RGB, [0xff_0000, 0xff00, 0xff, 0], "BGRX8"),
        61 => masked(8, DDPF_LUMINANCE, [0xff, 0, 0, 0], "R8"),
        65 => masked(8, DDPF_ALPHA, [0, 0, 0, 0xff], "A8"),
        _ => Err(Error::runtime(format!("'{name}': unsupported DXGI format {dxgi}"))),
    }
}

/// Decodes the top mip level of a DDS file. Returns the image and a short
/// description of the stored format.
pub fn decode_dds(bytes: &[u8], name: &str) -> Result<(Image, String)> {
    if bytes.len() < 128 || &bytes[0..4] != b"DDS " || get32(bytes, 4) != 124 {
        return Err(Error::runtime(format!("'{name}': not a valid DDS file")));
    }
    let h = 4;
    let height = get32(bytes, h + 8);
    let width = get32(bytes, h + 12);
    check_dimensions(width as i64, height as i64, name)?;
    let (width, height) = (width as i32, height as i32);
    let pf = h + 72;
    let pf_flags = get32(bytes, pf + 4);
    let fcc = get32(bytes, pf + 8);
    let mut offset = 128usize;
    let mut premultiplied = false;

    let (layout, desc) = if pf_flags & DDPF_FOURCC != 0 {
        use BlockFormat::*;
        if fcc == four_cc(b"DX10") {
            if bytes.len() < 148 {
                return Err(Error::runtime(format!("'{name}': truncated DX10 header")));
            }
            offset = 148;
            dxgi_layout(get32(bytes, 128), name)?
        } else {
            let (bf, d) = match &fcc.to_le_bytes() {
                b"DXT1" => (Bc1a, "DXT1"),
                b"DXT2" => (Bc2, "DXT2"),
                b"DXT3" => (Bc2, "DXT3"),
                b"DXT4" => (Bc3, "DXT4"),
                b"DXT5" => (Bc3, "DXT5"),
                b"ATI1" | b"BC4U" => (Bc4, "BC4"),
                b"ATI2" | b"BC5U" => (Bc5, "BC5"),
                other => {
                    // Show the FourCC like the C string it is meant to be,
                    // without control bytes that could mess up a terminal.
                    let shown: String = other
                        .iter()
                        .take_while(|&&c| c != 0)
                        .map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { '?' })
                        .collect();
                    return Err(Error::runtime(format!("'{name}': unsupported DDS FourCC '{shown}'")));
                }
            };
            premultiplied = d == "DXT2" || d == "DXT4";
            (Layout::Block(bf), d.to_string())
        }
    } else {
        let bits = get32(bytes, pf + 12);
        let masks = [get32(bytes, pf + 16), get32(bytes, pf + 20), get32(bytes, pf + 24), get32(bytes, pf + 28)];
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(Error::runtime(format!("'{name}': unsupported DDS bit depth {bits}")));
        }
        (Layout::Masked { bits, flags: pf_flags, masks }, format!("{bits}-bit uncompressed"))
    };

    let truncated = || Error::runtime(format!("'{name}': truncated DDS data"));
    let data = &bytes[offset..];
    let pixels = width as usize * height as usize;
    let rgba = match layout {
        Layout::Block(bf) => {
            let need = compressed_size(bf, width, height).ok_or_else(truncated)?;
            if data.len() < need {
                return Err(truncated());
            }
            decompress_surface(&data[..need], width, height, bf)?
        }
        Layout::Masked { bits, flags, masks } => {
            let bpp = (bits / 8) as usize;
            let need = pixels.checked_mul(bpp).ok_or_else(truncated)?;
            if data.len() < need {
                return Err(truncated());
            }
            let has_alpha = flags & (DDPF_ALPHAPIXELS | DDPF_ALPHA) != 0 && masks[3] != 0;
            let mut rgba = vec![0u8; pixels * 4];
            for (src, d) in data[..need].chunks_exact(bpp).zip(rgba.chunks_exact_mut(4)) {
                let mut word = [0u8; 4];
                word[..bpp].copy_from_slice(src);
                let v = u32::from_le_bytes(word);
                if flags & DDPF_LUMINANCE != 0 {
                    let l = extract_masked(v, masks[0]);
                    d[..3].fill(l);
                } else if flags & DDPF_ALPHA != 0 && flags & DDPF_RGB == 0 {
                    d[..3].fill(0);
                } else {
                    for c in 0..3 {
                        d[c] = extract_masked(v, masks[c]);
                    }
                }
                d[3] = if has_alpha { extract_masked(v, masks[3]) } else { 255 };
            }
            rgba
        }
    };

    let mut img = from_rgba8(&rgba, width, height);
    if premultiplied {
        unpremultiply(&mut img);
    }
    Ok((img, desc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_masked_handles_hostile_masks() {
        assert_eq!(extract_masked(0xff, 0xff), 255);
        assert_eq!(extract_masked(0x1f << 11, 0xf800), 255);
        assert_eq!(extract_masked(u32::MAX, u32::MAX), 255);
        // Non-contiguous mask: only the low run counts, never above 255.
        assert_eq!(extract_masked(0xffff_ffff, 0x0f0f), 255);
        assert_eq!(extract_masked(0, 0x8000_0000), 0);
    }

    #[test]
    fn codec_names() {
        assert_eq!(parse_dds_codec("DXT5n"), Some(DdsCodec::Bc3));
        assert_eq!(parse_dds_codec("bc7"), Some(DdsCodec::Bc7));
        assert_eq!(parse_dds_codec(""), None);
        assert_eq!(dds_codec_name(DdsCodec::Bc1a), "bc1a/dxt1a");
        assert_eq!(dds_codec_name(DdsCodec::Bc7), "bc7");
        assert_eq!(dds_codec_names()[0], "auto");
    }
}
