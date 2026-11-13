//! File format encoders/decoders and block compression.

mod common;

use common::{make_test_image, max_byte_diff, psnr, with_threads};
use idt3tc::img::bcn::{BlockFormat, BlockOptions, compress_surface, compressed_size};
use idt3tc::img::image::{Color, Image, to_byte, to_rgba8};
use idt3tc::img::io::{decode_image, encode_bmp, encode_jpg, encode_png};
use idt3tc::img::io_dds::{DdsCodec, DdsOptions, DdsResult, decode_dds, encode_dds};
use idt3tc::img::io_tga::{TgaOptions, encode_tga};

fn read_le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn decode(bytes: &[u8], name: &str) -> Image {
    decode_image(bytes, name).unwrap().0
}

fn dds_roundtrip(src: &Image, codec: DdsCodec) -> (Image, DdsResult) {
    let o = DdsOptions { codec, ..Default::default() };
    let r = encode_dds(src, &o).unwrap();
    let (out, _) = decode_dds(&r.bytes, "test.dds").unwrap();
    (out, r)
}

#[test]
fn tga_roundtrip_variants() {
    let src = make_test_image(33, 17, true);
    for rle in [false, true] {
        for top in [false, true] {
            let (bytes, bpp) = encode_tga(&src, &TgaOptions { rle, top_origin: top, ..Default::default() }).unwrap();
            assert_eq!(bpp, 32);
            assert_eq!(bytes[2], if rle { 10 } else { 2 });
            // Alpha bits and origin flag.
            assert_eq!(bytes[17] & 0x0f, 8);
            assert_eq!(bytes[17] & 0x20 != 0, top);
            let back = decode(&bytes, "t.tga");
            assert_eq!((back.width, back.height), (33, 17));
            assert_eq!(max_byte_diff(&src, &back), 0);
        }
    }
}

#[test]
fn tga_24_and_8_bit() {
    let src = make_test_image(16, 8, false);
    let (b24, bpp) = encode_tga(&src, &TgaOptions::default()).unwrap();
    assert_eq!(bpp, 24);
    assert_eq!(b24.len(), 18 + 16 * 8 * 3);
    assert_eq!(max_byte_diff(&src, &decode(&b24, "t.tga")), 0);

    let gray = Image::filled(4, 4, Color::new(0.5, 0.5, 0.5, 1.0));
    let (b8, bpp) = encode_tga(&gray, &TgaOptions { bpp: 8, ..Default::default() }).unwrap();
    assert_eq!(bpp, 8);
    assert_eq!(b8[2], 3);
    assert_eq!(max_byte_diff(&gray, &decode(&b8, "t.tga")), 0);
}

#[test]
fn tga_rle_runs_stay_in_scanline() {
    // A uniform image compresses to one run per row (idTech 3 decodes RLE
    // row by row and rejects packets crossing a scanline).
    let src = Image::filled(200, 3, Color::new(1.0, 0.0, 0.0, 1.0));
    let (b, _) = encode_tga(&src, &TgaOptions { rle: true, ..Default::default() }).unwrap();
    // Header, then per row a run of 128 and a run of 72, each 1 + 3 bytes.
    assert_eq!(b.len(), 18 + 3 * 8);
    assert_eq!(max_byte_diff(&src, &decode(&b, "t.tga")), 0);
}

#[test]
fn dds_bc_quality() {
    let rgb = make_test_image(64, 48, false);
    let rgba = make_test_image(64, 48, true);
    let cases = [
        (DdsCodec::Bc1, &rgb, 7, 30.0),
        (DdsCodec::Bc2, &rgba, 15, 28.0),
        (DdsCodec::Bc3, &rgba, 15, 30.0),
        (DdsCodec::Bc4, &rgb, 1, 38.0),
        (DdsCodec::Bc5, &rgb, 3, 36.0),
        (DdsCodec::Bc7, &rgba, 15, 38.0),
    ];
    for (codec, img, mask, min_psnr) in cases {
        let (out, _) = dds_roundtrip(img, codec);
        assert_eq!((out.width, out.height), (64, 48));
        let p = psnr(img, &out, mask);
        assert!(p >= min_psnr, "{codec:?}: PSNR {p:.2} dB < {min_psnr:.1}");
    }
}

#[test]
fn dds_uncompressed_exact() {
    let src = make_test_image(13, 7, true);
    for c in [DdsCodec::Rgba8, DdsCodec::Bgra8] {
        assert_eq!(max_byte_diff(&src, &dds_roundtrip(&src, c).0), 0);
    }
    let opaque = make_test_image(13, 7, false);
    for c in [DdsCodec::Bgr8, DdsCodec::Bgrx8] {
        assert_eq!(max_byte_diff(&opaque, &dds_roundtrip(&opaque, c).0), 0);
    }
}

#[test]
fn dds_bc1a_binary_alpha() {
    let mut src = make_test_image(32, 32, false);
    for y in 0..32 {
        for x in 0..32 {
            src.pixel_mut(x, y)[3] = if (x / 3 + y / 5) % 2 != 0 { 1.0 } else { 0.0 };
        }
    }
    let (out, _) = dds_roundtrip(&src, DdsCodec::Bc1a);
    for (a, b) in src.data.iter().zip(&out.data).skip(3).step_by(4) {
        assert_eq!(to_byte(*a), to_byte(*b));
    }
}

#[test]
fn dds_bc1_opaque_never_transparent() {
    // Content that tempts an encoder into 3-color mode (black texels).
    let mut src = make_test_image(32, 32, false);
    for y in (0..32).step_by(2) {
        for x in (0..32).step_by(3) {
            src.pixel_mut(x, y)[..3].fill(0.0);
        }
    }
    let (out, _) = dds_roundtrip(&src, DdsCodec::Bc1);
    assert!(out.data.iter().skip(3).step_by(4).all(|&a| to_byte(a) == 255));
}

#[test]
fn dds_auto_codec_and_headers() {
    let (_, r) = dds_roundtrip(&make_test_image(64, 32, false), DdsCodec::Auto);
    assert_eq!(r.codec, DdsCodec::Bc1);
    assert_eq!(r.mip_levels, 7);
    assert_eq!(read_le32(&r.bytes, 0), 0x20534444); // "DDS "
    assert_eq!(read_le32(&r.bytes, 28), 7); // dwMipMapCount
    assert_eq!(read_le32(&r.bytes, 84), 0x31545844); // "DXT1"
    // 64x32 BC1 chain: blocks are at least 4x4.
    assert_eq!(r.bytes.len(), 128 + 1024 + 256 + 64 + 16 + 8 + 8 + 8);

    let (_, r) = dds_roundtrip(&make_test_image(16, 16, true), DdsCodec::Auto);
    assert_eq!(r.codec, DdsCodec::Bc3);

    let (_, r) = dds_roundtrip(&make_test_image(16, 16, true), DdsCodec::Bc7);
    assert!(r.dx10);
    assert_eq!(read_le32(&r.bytes, 84), 0x30315844); // "DX10"
    assert_eq!(read_le32(&r.bytes, 128), 98); // DXGI_FORMAT_BC7_UNORM
}

#[test]
fn dds_mip_options() {
    let mut o = DdsOptions { codec: DdsCodec::Bc3, ..Default::default() };
    o.mips.levels = 1;
    assert_eq!(encode_dds(&make_test_image(64, 64, true), &o).unwrap().mip_levels, 1);
    o.mips.levels = 0;
    o.mips.min_size = 8;
    assert_eq!(encode_dds(&make_test_image(64, 64, true), &o).unwrap().mip_levels, 4);
}

#[test]
fn dds_invalid_combinations() {
    let o = DdsOptions { codec: DdsCodec::Bc7, dx10: 0, ..Default::default() };
    assert!(encode_dds(&make_test_image(8, 8, true), &o).unwrap_err().is_usage());
    let o = DdsOptions { codec: DdsCodec::Bc4, srgb: true, ..Default::default() };
    assert!(encode_dds(&make_test_image(8, 8, true), &o).unwrap_err().is_usage());
}

#[test]
fn bcn_threads_match() {
    let src = make_test_image(70, 45, true);
    let rgba = to_rgba8(&src);
    for f in [BlockFormat::Bc1, BlockFormat::Bc3, BlockFormat::Bc5, BlockFormat::Bc7] {
        let o = BlockOptions { quality: 0.2, ..Default::default() };
        let a = with_threads(1, || compress_surface(&rgba, 70, 45, f, &o).unwrap());
        let b = with_threads(8, || compress_surface(&rgba, 70, 45, f, &o).unwrap());
        assert!(a == b);
        assert_eq!(Some(a.len()), compressed_size(f, 70, 45));
    }
}

#[test]
fn bcn_odd_sizes() {
    // 1xN strips and sizes that are no multiple of the block size.
    for (w, h) in [(1, 1), (1, 9), (9, 1), (5, 3), (3, 5)] {
        let src = make_test_image(w, h, true);
        for c in [DdsCodec::Bc1, DdsCodec::Bc1a, DdsCodec::Bc3, DdsCodec::Bc4, DdsCodec::Bc5, DdsCodec::Bc7] {
            let (out, r) = dds_roundtrip(&src, c);
            assert_eq!((out.width, out.height), (w, h), "{c:?} {w}x{h}");
            assert!(r.mip_levels >= 1);
        }
    }
    // A buffer that does not match the size is refused.
    assert!(compress_surface(&[0; 10], 4, 4, BlockFormat::Bc1, &BlockOptions::default()).is_err());
}

#[test]
fn png_jpg_bmp_roundtrip() {
    let src = make_test_image(31, 9, true);
    assert_eq!(max_byte_diff(&src, &decode(&encode_png(&src, 0, 6).unwrap(), "t.png")), 0);
    let opaque = make_test_image(31, 9, false);
    assert_eq!(max_byte_diff(&opaque, &decode(&encode_bmp(&opaque, 0).unwrap(), "t.bmp")), 0);
    assert!(psnr(&opaque, &decode(&encode_jpg(&opaque, 95).unwrap(), "t.jpg"), 7) > 30.0);
}

#[test]
fn png_channel_counts_and_levels() {
    let src = make_test_image(9, 4, true);
    for channels in 1..=4 {
        for level in [0, 1, 9] {
            let (back, info) = decode_image(&encode_png(&src, channels, level).unwrap(), "t.png").unwrap();
            assert_eq!(info.channels, channels);
            assert_eq!((back.width, back.height), (9, 4));
        }
    }
}
