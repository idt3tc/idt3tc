//! Loading of all supported input formats and the PNG, JPEG and BMP
//! encoders. TGA and DDS live in io_tga / io_dds.
//!
//! Decoding runs on untrusted files: the format is detected from the
//! content, dimensions are checked against the global limits before any
//! pixel buffer is allocated, and the image crate decoders get allocation
//! limits on top.

use std::io::Cursor;

use image::{ColorType, DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};

use super::image::{Image, MAX_PIXELS, analyze_alpha, check_dimensions, from_rgba8, luma, to_byte};
use super::io_dds::decode_dds;
use super::io_psd::{decode_psd, is_psd};
use crate::common::fs;
use crate::common::{Error, Result};

/// What was found in a loaded file.
#[derive(Debug, Clone, Default)]
pub struct LoadInfo {
    /// "png", "tga", "dds (DXT5)", ...
    pub format: String,
    /// Channels stored in the file.
    pub channels: i32,
    /// 8 or 16.
    pub bits_per_channel: i32,
}

/// Extensions (lowercase, no dot) treated as images when scanning folders.
const IMAGE_EXTENSIONS: &[&str] =
    &["png", "jpg", "jpeg", "tga", "bmp", "psd", "gif", "hdr", "pnm", "ppm", "pgm", "pbm", "pam", "dds"];

pub fn is_image_extension(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext)
}

/// Detects the container from magic bytes. TGA has no signature and is
/// the fallback, as in most image libraries.
fn sniff_format(b: &[u8]) -> ImageFormat {
    if b.starts_with(b"\x89PNG") {
        ImageFormat::Png
    } else if b.starts_with(&[0xff, 0xd8]) {
        ImageFormat::Jpeg
    } else if b.starts_with(b"BM") {
        ImageFormat::Bmp
    } else if b.starts_with(b"GIF8") {
        ImageFormat::Gif
    } else if b.starts_with(b"#?RADIANCE") || b.starts_with(b"#?RGBE") {
        ImageFormat::Hdr
    } else if b.len() >= 2 && b[0] == b'P' && (b'1'..=b'7').contains(&b[1]) {
        ImageFormat::Pnm
    } else {
        ImageFormat::Tga
    }
}

/// Short format name for messages: derived from the content when it has a
/// signature, else from the file extension.
fn format_label(b: &[u8], name: &str) -> String {
    let by_magic = if b.starts_with(b"\x89PNG") {
        "png"
    } else if b.starts_with(&[0xff, 0xd8]) {
        "jpeg"
    } else if b.starts_with(b"BM") {
        "bmp"
    } else if is_psd(b) {
        "psd"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else {
        ""
    };
    if !by_magic.is_empty() {
        return by_magic.to_string();
    }
    let ext = fs::extension(name);
    if ext.is_empty() { "image".to_string() } else { ext }
}

/// Limits for the image crate decoders. Dimensions are checked separately
/// (with our own message); the allocation cap covers the largest legal
/// image at 16 bits per channel plus decoder overhead.
fn decode_limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = None;
    l.max_image_height = None;
    l.max_alloc = Some(MAX_PIXELS * 8 + (256 << 20));
    l
}

fn channels_of(c: ExtendedColorType) -> i32 {
    c.channel_count() as i32
}

/// Converts an HDR value to 8 bits the way stb_image does for LDR loads
/// (gamma 1/2.2, scale 1), keeping 8-bit results identical to the
/// previous implementation.
fn hdr_to_byte(v: f32) -> u8 {
    let z = (libm::pow(v as f64, (1.0f32 / 2.2f32) as f64) as f32) * 255.0 + 0.5;
    // NaN and negative values end up as 0, like stb's clamping.
    z.clamp(0.0, 255.0) as u8
}

fn decode_with_image_crate(bytes: &[u8], name: &str, fmt: ImageFormat) -> Result<(Image, i32, i32)> {
    let fail = |what: &str, e: image::ImageError| Error::runtime(format!("'{name}': {what} ({e})"));
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(fmt);
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(|e| fail("unsupported or corrupt image", e))?;
    let (w, h) = decoder.dimensions();
    check_dimensions(w as i64, h as i64, name)?;
    decoder.set_limits(decode_limits()).map_err(|e| fail("decoding failed", e))?;
    let channels = channels_of(decoder.original_color_type());
    let color = decoder.color_type();
    let dynimg = DynamicImage::from_decoder(decoder).map_err(|e| fail("decoding failed", e))?;
    let (w, h) = (w as i32, h as i32);
    match color {
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => {
            // Keep the extra precision of 16-bit sources.
            let px = dynimg.into_rgba16();
            let mut img = Image::new(w, h);
            for (d, s) in img.data.iter_mut().zip(px.as_raw()) {
                *d = *s as f32 / 65535.0;
            }
            Ok((img, channels, 16))
        }
        ColorType::Rgb32F | ColorType::Rgba32F => {
            let px = dynimg.into_rgba32f();
            let rgba: Vec<u8> = px
                .as_raw()
                .chunks_exact(4)
                .flat_map(|p| [hdr_to_byte(p[0]), hdr_to_byte(p[1]), hdr_to_byte(p[2]), to_byte(p[3])])
                .collect();
            Ok((from_rgba8(&rgba, w, h), channels, 8))
        }
        _ => {
            let px = dynimg.into_rgba8();
            Ok((from_rgba8(px.as_raw(), w, h), channels, 8))
        }
    }
}

pub fn decode_image(bytes: &[u8], name: &str) -> Result<(Image, LoadInfo)> {
    if bytes.starts_with(b"DDS ") {
        let (img, desc) = decode_dds(bytes, name)?;
        return Ok((img, LoadInfo { format: format!("dds ({desc})"), channels: 4, bits_per_channel: 8 }));
    }
    if bytes.len() as u64 > fs::MAX_FILE_SIZE {
        return Err(Error::runtime(format!("'{name}': file too large")));
    }
    if bytes.is_empty() {
        return Err(Error::runtime(format!("'{name}': unsupported or corrupt image (empty file)")));
    }
    let (img, channels, bits) = if is_psd(bytes) {
        let (img, bits) = decode_psd(bytes, name)?;
        // The merged image always decodes to RGBA, as stb_image reported.
        (img, 4, bits)
    } else {
        decode_with_image_crate(bytes, name, sniff_format(bytes))?
    };
    let info = LoadInfo { format: format_label(bytes, name), channels, bits_per_channel: bits };
    Ok((img, info))
}

pub fn load_image(path: &str) -> Result<(Image, LoadInfo)> {
    decode_image(&fs::read_file(path)?, path)
}

/// Interleaved 8-bit buffer with `channels` channels (1: luma, 2: luma +
/// alpha, 3: RGB, 4: RGBA).
fn pack_channels(img: &Image, channels: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.pixel_count() * channels);
    for p in img.data.chunks_exact(4) {
        match channels {
            1 => out.push(to_byte(luma(p))),
            2 => out.extend_from_slice(&[to_byte(luma(p)), to_byte(p[3])]),
            _ => out.extend(p[..channels].iter().map(|&v| to_byte(v))),
        }
    }
    out
}

fn auto_channels(img: &Image, channels: i32) -> i32 {
    if channels > 0 {
        channels
    } else if analyze_alpha(img).opaque {
        3
    } else {
        4
    }
}

/// channels: 0 = auto (4 if the image has alpha, else 3), or 1..4.
/// compression_level: 0 (store) .. 9.
pub fn encode_png(img: &Image, channels: i32, compression_level: i32) -> Result<Vec<u8>> {
    let channels = auto_channels(img, channels);
    let px = pack_channels(img, channels as usize);
    let mut out = Vec::new();
    let fail = |e: png::EncodingError| Error::runtime(format!("PNG encoding failed ({e})"));
    {
        let mut enc = png::Encoder::new(&mut out, img.width as u32, img.height as u32);
        enc.set_color(match channels {
            1 => png::ColorType::Grayscale,
            2 => png::ColorType::GrayscaleAlpha,
            3 => png::ColorType::Rgb,
            _ => png::ColorType::Rgba,
        });
        enc.set_depth(png::BitDepth::Eight);
        enc.set_deflate_compression(match compression_level {
            ..=0 => png::DeflateCompression::NoCompression,
            n => png::DeflateCompression::Level(n.min(9) as u8),
        });
        let mut writer = enc.write_header().map_err(fail)?;
        writer.write_image_data(&px).map_err(fail)?;
        writer.finish().map_err(fail)?;
    }
    Ok(out)
}

pub fn encode_jpg(img: &Image, quality: i32) -> Result<Vec<u8>> {
    // JPEG has no alpha channel.
    let px = pack_channels(img, 3);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100) as u8)
        .write_image(&px, img.width as u32, img.height as u32, ExtendedColorType::Rgb8)
        .map_err(|e| Error::runtime(format!("JPEG encoding failed ({e})")))?;
    Ok(out)
}

pub fn encode_bmp(img: &Image, channels: i32) -> Result<Vec<u8>> {
    let channels = auto_channels(img, channels);
    if channels != 3 && channels != 4 {
        return Err(Error::usage("BMP output supports 3 or 4 channels only"));
    }
    let px = pack_channels(img, channels as usize);
    let mut out = Vec::new();
    let color = if channels == 3 { ExtendedColorType::Rgb8 } else { ExtendedColorType::Rgba8 };
    image::codecs::bmp::BmpEncoder::new(&mut out)
        .write_image(&px, img.width as u32, img.height as u32, color)
        .map_err(|e| Error::runtime(format!("BMP encoding failed ({e})")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_headers_fail_cleanly() {
        // TGA claiming 65535x65535 with no data.
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        tga[12..16].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        tga[16] = 24;
        assert!(decode_image(&tga, "t.tga").is_err());
        // Empty, random and truncated files.
        assert!(decode_image(&[], "t.png").is_err());
        assert!(decode_image(b"\x89PNG\r\n\x1a\n", "t.png").is_err());
        assert!(decode_image(b"BM", "t.bmp").is_err());
        assert!(decode_image(b"P6\n100000 100000\n255\n", "t.ppm").is_err());
        assert!(decode_image(b"DDS ", "t.dds").is_err());
    }

    #[test]
    fn hdr_conversion_matches_stb() {
        assert_eq!(hdr_to_byte(0.0), 0);
        assert_eq!(hdr_to_byte(1.0), 255);
        assert_eq!(hdr_to_byte(100.0), 255);
        assert_eq!(hdr_to_byte(-1.0), 0);
        assert_eq!(hdr_to_byte(f32::NAN), 0);
        assert_eq!(hdr_to_byte(0.5), 186);
    }
}
