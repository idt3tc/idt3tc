//! Crafted and corrupted input files. Every decoder must turn them into an
//! ordinary error (or a valid image) without panicking, hanging or
//! allocating memory based on unvalidated header fields.

mod common;

use common::make_test_image;
use idt3tc::img::image::Image;
use idt3tc::img::io::{decode_image, encode_bmp, encode_jpg, encode_png};
use idt3tc::img::io_dds::{DdsCodec, DdsOptions, encode_dds};
use idt3tc::img::io_tga::{TgaOptions, encode_tga};

fn put16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn put32be(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

/// Decodes `bytes` and checks that the outcome is an error mentioning
/// the file name, or an image within the limits.
fn decode_ok_or_clean_error(bytes: &[u8], what: &str) -> Option<Image> {
    match decode_image(bytes, "crafted.bin") {
        Ok((img, _)) => {
            assert!(img.width >= 1 && img.height >= 1, "{what}");
            assert_eq!(img.data.len(), img.width as usize * img.height as usize * 4, "{what}");
            Some(img)
        }
        Err(e) => {
            assert!(!e.is_usage(), "{what}: {e}");
            assert!(e.message().contains("crafted.bin"), "{what}: {}", e.message());
            None
        }
    }
}

fn expect_error(bytes: &[u8], what: &str) {
    assert!(decode_ok_or_clean_error(bytes, what).is_none(), "{what}: decoded successfully");
}

/// A minimal DDS header ("DDS " + 124 bytes) with a FourCC pixel format.
fn dds_header(w: u32, h: u32, mips: u32, fourcc: &[u8; 4]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(b"DDS ");
    put32(&mut b, 4, 124);
    put32(&mut b, 8, 0x1 | 0x2 | 0x4 | 0x1000 | 0x20000);
    put32(&mut b, 12, h);
    put32(&mut b, 16, w);
    put32(&mut b, 28, mips);
    put32(&mut b, 76, 32);
    put32(&mut b, 80, 0x4); // DDPF_FOURCC
    b[84..88].copy_from_slice(fourcc);
    put32(&mut b, 108, 0x1000);
    b
}

fn dds_dx10(w: u32, h: u32, dxgi: u32, array: u32) -> Vec<u8> {
    let mut b = dds_header(w, h, 1, b"DX10");
    b.extend_from_slice(&[0; 20]);
    put32(&mut b, 128, dxgi);
    put32(&mut b, 132, 3); // TEXTURE2D
    put32(&mut b, 140, array);
    b
}

#[test]
fn dds_crafted_headers() {
    let block = [0x55u8; 4096];
    let with = |mut h: Vec<u8>, data: &[u8]| {
        h.extend_from_slice(data);
        h
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("magic only", b"DDS ".to_vec()),
        ("truncated header", dds_header(4, 4, 1, b"DXT1")[..100].to_vec()),
        ("no data", dds_header(4, 4, 1, b"DXT1")),
        ("zero size", with(dds_header(0, 0, 1, b"DXT1"), &block)),
        ("zero width", with(dds_header(0, 4, 1, b"DXT1"), &block)),
        ("huge", with(dds_header(0x7fff_ffff, 0x7fff_ffff, 1, b"DXT1"), &block)),
        ("max u32", with(dds_header(u32::MAX, u32::MAX, u32::MAX, b"DXT5"), &block)),
        ("over limit", with(dds_header(65537, 4, 1, b"DXT1"), &block)),
        ("pixel bomb", with(dds_header(65536, 65536, 1, b"DXT1"), &block)),
        ("short data", with(dds_header(64, 64, 1, b"DXT5"), &block[..100])),
        ("bogus fourcc", with(dds_header(4, 4, 1, b"ABCD"), &block)),
        ("zero fourcc", with(dds_header(4, 4, 1, &[0; 4]), &block)),
        ("dx10 short", dds_header(4, 4, 1, b"DX10")),
        ("dx10 bogus dxgi", with(dds_dx10(4, 4, 9999, 1), &block)),
        ("dx10 zero dxgi", with(dds_dx10(4, 4, 0, 1), &block)),
        ("dx10 bc6h", with(dds_dx10(4, 4, 95, 1), &block)),
    ];
    for (what, bytes) in &cases {
        expect_error(bytes, what);
    }
    // Absurd mip and array counts must not matter for the top level image,
    // which is the only one read.
    decode_ok_or_clean_error(&with(dds_dx10(4, 4, 98, u32::MAX), &block), "array size");
    decode_ok_or_clean_error(&with(dds_header(4, 4, u32::MAX, b"DXT1"), &block), "mip count");
}

#[test]
fn dds_crafted_uncompressed_formats() {
    let data = [0xa5u8; 16 * 16 * 4];
    for (bits, masks) in [
        (32, [0, 0, 0, 0]),
        (32, [0xffff_ffff; 4]),
        (24, [0xff, 0xff00, 0xff_0000, 0xff00_0000]),
        (16, [0xf800, 0x07e0, 0x001f, 0]),
        (8, [0xff, 0, 0, 0]),
        (0, [0xff, 0xff00, 0xff_0000, 0]),
        (7, [0x7f, 0, 0, 0]),
        (64, [u32::MAX; 4]),
        (u32::MAX, [1, 2, 4, 8]),
    ] {
        for pitch in [0u32, 1, 64, u32::MAX] {
            let mut b = dds_header(16, 16, 1, &[0; 4]);
            put32(&mut b, 80, 0x40 | 0x1); // DDPF_RGB | DDPF_ALPHAPIXELS
            put32(&mut b, 20, pitch);
            put32(&mut b, 88, bits);
            for (i, m) in masks.iter().enumerate() {
                put32(&mut b, 92 + 4 * i, *m);
            }
            b.extend_from_slice(&data);
            decode_ok_or_clean_error(&b, &format!("{bits} bit, masks {masks:x?}, pitch {pitch}"));
        }
    }
}

fn tga_header(kind: u8, w: u16, h: u16, bpp: u8, desc: u8) -> Vec<u8> {
    let mut b = vec![0u8; 18];
    b[2] = kind;
    put16(&mut b, 12, w);
    put16(&mut b, 14, h);
    b[16] = bpp;
    b[17] = desc;
    b
}

#[test]
fn tga_crafted_headers() {
    let mut huge_rle = tga_header(10, 65535, 65535, 32, 8);
    // One run packet repeating a single pixel 128 times, then nothing.
    huge_rle.extend_from_slice(&[0xff, 1, 2, 3, 4]);
    let mut overlong_run = tga_header(10, 3, 1, 24, 0);
    overlong_run.extend_from_slice(&[0xff, 1, 2, 3]);
    let mut cmap = tga_header(1, 4, 4, 8, 0);
    // Color map spec: first entry 0xffff, 0xffff entries of 32 bits.
    put16(&mut cmap, 3, 0xffff);
    put16(&mut cmap, 5, 0xffff);
    cmap[7] = 32;
    let mut id = tga_header(2, 2, 2, 24, 0);
    id[0] = 255;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("header only", tga_header(2, 4, 4, 32, 8)),
        ("truncated", tga_header(2, 4, 4, 32, 8)[..10].to_vec()),
        ("zero size", tga_header(2, 0, 0, 32, 8)),
        ("huge rle", huge_rle),
        ("huge raw", tga_header(2, 65535, 65535, 32, 8)),
        ("bogus depth", tga_header(2, 4, 4, 13, 0)),
        ("bogus type", tga_header(77, 4, 4, 32, 8)),
        ("color map overflow", cmap),
        ("id field past end", id),
    ];
    for (what, bytes) in &cases {
        expect_error(bytes, what);
    }
    // A run longer than the image is either clipped or an error.
    decode_ok_or_clean_error(&overlong_run, "overlong run");
}

/// PSD header: signature, version 1, channels, height, width, depth, mode.
fn psd(channels: u16, w: u32, h: u32, depth: u16, mode: u16, rle: bool, data: &[u8]) -> Vec<u8> {
    let mut b = b"8BPS".to_vec();
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&[0; 6]);
    b.extend_from_slice(&channels.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&depth.to_be_bytes());
    b.extend_from_slice(&mode.to_be_bytes());
    // Empty color mode data, image resources and layer/mask sections.
    b.extend_from_slice(&[0; 12]);
    b.extend_from_slice(&(rle as u16).to_be_bytes());
    b.extend_from_slice(data);
    b
}

/// A valid 4x4 RGB PSD, raw or PackBits compressed.
fn valid_psd(rle: bool) -> Vec<u8> {
    let mut data = Vec::new();
    if rle {
        // Per row byte counts (3 channels x 4 rows), then literal packets.
        data.extend(std::iter::repeat_n(5u16.to_be_bytes(), 12).flatten());
        for i in 0..12u8 {
            data.extend_from_slice(&[3, i, i * 2, i * 3, i * 4]);
        }
    } else {
        data.extend((0..48u8).map(|i| i * 5));
    }
    psd(3, 4, 4, 8, 3, rle, &data)
}

#[test]
fn psd_crafted_headers() {
    for rle in [false, true] {
        assert!(decode_ok_or_clean_error(&valid_psd(rle), "valid psd").is_some());
    }
    let mut bad_counts = valid_psd(true);
    // First row claims 65535 bytes.
    let off = 26 + 12 + 2;
    bad_counts[off..off + 2].copy_from_slice(&[0xff, 0xff]);
    let mut bad_section = valid_psd(false);
    // Color mode data section claims 4 GiB.
    put32be(&mut bad_section, 26, u32::MAX);
    let mut runaway = psd(3, 4, 4, 8, 3, true, &[]);
    runaway.extend(std::iter::repeat_n(5u16.to_be_bytes(), 12).flatten());
    // Repeat packets that write far past the end of each row.
    runaway.extend(std::iter::repeat_n([0x81u8, 7], 64).flatten());
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("signature only", b"8BPS".to_vec()),
        ("header only", psd(3, 4, 4, 8, 3, false, &[])),
        ("huge", psd(3, 300_000, 300_000, 8, 3, false, &[0; 64])),
        ("max dims", psd(3, u32::MAX, u32::MAX, 8, 3, true, &[0; 64])),
        ("pixel bomb", psd(3, 30_000, 30_000, 8, 3, true, &[0; 64])),
        ("zero channels", psd(0, 4, 4, 8, 3, false, &[0; 64])),
        ("too many channels", psd(65535, 4, 4, 8, 3, false, &[0; 64])),
        ("bogus depth", psd(3, 4, 4, 7, 3, false, &[0; 64])),
        ("32 bit", psd(3, 4, 4, 32, 3, false, &[0; 256])),
        ("bogus compression", {
            let mut b = valid_psd(false);
            b[38] = 9;
            b
        }),
        ("bad row counts", bad_counts),
        ("bad section length", bad_section),
        ("runaway packbits", runaway),
    ];
    for (what, bytes) in &cases {
        decode_ok_or_clean_error(bytes, what);
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Rewrites the IHDR size of a PNG and fixes up the chunk CRC.
fn png_with_size(png: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut b = png.to_vec();
    put32be(&mut b, 16, w);
    put32be(&mut b, 20, h);
    let crc = crc32(&b[12..29]);
    put32be(&mut b, 29, crc);
    b
}

#[test]
fn image_crate_formats_with_extreme_sizes() {
    let png = encode_png(&make_test_image(4, 4, true), 4, 6).unwrap();
    assert!(decode_ok_or_clean_error(&png_with_size(&png, 4, 4), "unchanged png").is_some());
    for (w, h) in [(65537, 1), (1, 65537), (65536, 65536), (0x7fff_ffff, 0x7fff_ffff), (30000, 30000)] {
        expect_error(&png_with_size(&png, w, h), &format!("png {w}x{h}"));
    }
    let mut bmp = encode_bmp(&make_test_image(4, 4, false), 3).unwrap();
    put32(&mut bmp, 18, 0x7fff_ffff);
    put32(&mut bmp, 22, 0x8000_0000); // i32::MIN height
    expect_error(&bmp, "bmp huge");
}

/// Valid files of every supported format, small enough to mutate.
fn sample_files() -> Vec<(String, Vec<u8>)> {
    let rgba = make_test_image(9, 7, true);
    let rgb = make_test_image(9, 7, false);
    let mut files = vec![
        ("png".to_string(), encode_png(&rgba, 0, 6).unwrap()),
        ("bmp".to_string(), encode_bmp(&rgb, 0).unwrap()),
        ("jpg".to_string(), encode_jpg(&rgb, 80).unwrap()),
        ("psd raw".to_string(), valid_psd(false)),
        ("psd rle".to_string(), valid_psd(true)),
    ];
    for (rle, bpp) in [(false, 32), (true, 32), (true, 24), (true, 8)] {
        let o = TgaOptions { rle, bpp, ..Default::default() };
        files.push((format!("tga {bpp} rle={rle}"), encode_tga(&rgba, &o).unwrap().0));
    }
    for codec in [DdsCodec::Bc1, DdsCodec::Bc3, DdsCodec::Bc4, DdsCodec::Bc5, DdsCodec::Bc7, DdsCodec::Bgr8, DdsCodec::Rgba8] {
        let mut o = DdsOptions { codec, ..Default::default() };
        o.block.quality = 0.05;
        files.push((format!("dds {codec:?}"), encode_dds(&rgba, &o).unwrap().bytes));
    }
    files
}

#[test]
fn truncated_files() {
    for (what, bytes) in sample_files() {
        for len in 0..bytes.len() {
            decode_ok_or_clean_error(&bytes[..len], &format!("{what} truncated to {len}"));
        }
    }
}

#[test]
fn mutated_files() {
    // Deterministic xorshift so failures are reproducible.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for (what, bytes) in sample_files() {
        for round in 0..600 {
            let mut m = bytes.clone();
            // Mostly hit the headers, where the sizes and offsets live.
            let span = if round % 2 == 0 { m.len().min(160) } else { m.len() };
            for _ in 0..1 + next() % 4 {
                let i = (next() % span as u64) as usize;
                m[i] = match next() % 4 {
                    0 => 0,
                    1 => 0xff,
                    2 => m[i] ^ (1 << (next() % 8)),
                    _ => next() as u8,
                };
            }
            decode_ok_or_clean_error(&m, &format!("{what} round {round}"));
        }
    }
}
