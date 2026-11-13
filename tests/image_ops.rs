//! Resampling, geometric transforms and registered operations.

mod common;

use common::{context_with, make_test_image, max_byte_diff, run_op, with_threads};
use idt3tc::img::image::{Color, Image, clamp_image};
use idt3tc::img::mipmap::{MipOptions, alpha_coverage, build_mip_chain, full_mip_count};
use idt3tc::img::resample::{Filter, ResampleOptions, resample};
use idt3tc::img::sampling::{EdgeMode, Interp};
use idt3tc::img::transform::{AffineOptions, rotate_arbitrary, rotate_quarter, translate};

fn near(a: f32, b: f64, eps: f64) -> bool {
    (a as f64 - b).abs() <= eps
}

#[test]
fn resample_identity() {
    let src = make_test_image(37, 21, true);
    let o = ResampleOptions { filter: Filter::Lanczos3, ..Default::default() };
    assert_eq!(max_byte_diff(&src, &resample(&src, 37, 21, &o).unwrap()), 0);
}

#[test]
fn resample_box_halving_averages() {
    let mut src = Image::new(2, 2);
    for (p, v) in src.data.chunks_exact_mut(4).zip([0.0, 1.0, 0.5, 0.5]) {
        p.copy_from_slice(&[v, v, v, 1.0]);
    }
    let o = ResampleOptions { filter: Filter::Box, ..Default::default() };
    assert!(near(resample(&src, 1, 1, &o).unwrap().data[0], 0.5, 1e-5));
}

#[test]
fn resample_threads_match() {
    let src = make_test_image(129, 77, true);
    let o = ResampleOptions { filter: Filter::Mitchell, ..Default::default() };
    let a = with_threads(1, || resample(&src, 50, 200, &o).unwrap());
    let b = with_threads(8, || resample(&src, 50, 200, &o).unwrap());
    assert!(a.data == b.data);
}

#[test]
fn rotate_quarter_roundtrip() {
    let src = make_test_image(5, 3, true);
    let r = rotate_quarter(&src, 1);
    assert_eq!((r.width, r.height), (3, 5));
    // Clockwise: the top-left pixel ends up at the top-right.
    assert_eq!(src.pixel(0, 0), r.pixel(2, 0));
    let back = rotate_quarter(&rotate_quarter(&rotate_quarter(&r, 1), 1), 1);
    assert!(back.data == src.data);
}

#[test]
fn rotate_arbitrary_90_matches_quarter() {
    let src = make_test_image(16, 16, false);
    let o = AffineOptions { interp: Interp::Bilinear, ..Default::default() };
    let a = rotate_arbitrary(&src, 90.0, true, &o).unwrap();
    let b = rotate_quarter(&src, 1);
    assert_eq!((a.width, a.height), (b.width, b.height));
    assert!(max_byte_diff(&a, &b) <= 1);
}

#[test]
fn translate_wraps() {
    let src = make_test_image(8, 4, true);
    let o = AffineOptions { edge: EdgeMode::Repeat, ..Default::default() };
    let out = translate(&src, 3.0, -1.0, &o);
    for y in 0..4 {
        for x in 0..8 {
            assert_eq!(out.pixel(x, y), src.pixel((x - 3 + 8) % 8, (y + 1) % 4));
        }
    }
}

#[test]
fn op_order_matters() {
    // crop then resize != resize then crop
    let src = make_test_image(64, 64, false);
    let (mut a, mut b) = (context_with(src.clone()), context_with(src));
    run_op(&mut a, "crop", "32x32", &[]).unwrap();
    run_op(&mut a, "resize", "16x16", &[]).unwrap();
    run_op(&mut b, "resize", "16x16", &[]).unwrap();
    run_op(&mut b, "crop", "8x8", &[]).unwrap();
    assert_eq!(a.image.width, 16);
    assert_eq!(b.image.width, 8);
}

#[test]
fn op_pot() {
    let mut ctx = context_with(make_test_image(300, 200, false));
    run_op(&mut ctx, "pot", "", &[]).unwrap();
    assert_eq!((ctx.image.width, ctx.image.height), (256, 256));

    let mut up = context_with(make_test_image(300, 70, false));
    run_op(&mut up, "pot", "", &[("mode", "up"), ("max", "256")]).unwrap();
    assert_eq!((up.image.width, up.image.height), (256, 128));
}

#[test]
fn op_trim_alpha() {
    let mut src = Image::filled(20, 10, Color::TRANSPARENT);
    for y in 3..6 {
        for x in 5..12 {
            src.pixel_mut(x, y)[3] = 1.0;
        }
    }
    let mut ctx = context_with(src);
    run_op(&mut ctx, "trim", "", &[]).unwrap();
    assert_eq!((ctx.image.width, ctx.image.height), (7, 3));
}

#[test]
fn op_canvas_and_pad() {
    let mut ctx = context_with(make_test_image(10, 10, false));
    run_op(&mut ctx, "canvas", "16x12", &[]).unwrap();
    assert_eq!((ctx.image.width, ctx.image.height), (16, 12));
    run_op(&mut ctx, "pad", "1,2", &[]).unwrap();
    assert_eq!((ctx.image.width, ctx.image.height), (18, 16));
}

#[test]
fn op_color_basics() {
    let mut ctx = context_with(Image::filled(2, 2, Color::new(0.2, 0.4, 0.6, 1.0)));
    run_op(&mut ctx, "invert", "", &[]).unwrap();
    assert!(near(ctx.image.data[0], 0.8, 1e-6));
    // Alpha is untouched by default.
    assert!(near(ctx.image.data[3], 1.0, 1e-6));
    run_op(&mut ctx, "swizzle", "bgr1", &[]).unwrap();
    assert!(near(ctx.image.data[0], 0.4, 1e-6));
    run_op(&mut ctx, "grayscale", "", &[]).unwrap();
    assert!(near(ctx.image.data[0], ctx.image.data[1] as f64, 1e-6));
    run_op(&mut ctx, "alpha", "set", &[("value", "0.25")]).unwrap();
    assert!(near(ctx.image.data[3], 0.25, 1e-6));
}

#[test]
fn op_colorkey() {
    let mut src = Image::filled(2, 1, Color::new(1.0, 0.0, 1.0, 1.0));
    // White.
    src.pixel_mut(1, 0)[1] = 1.0;
    let mut ctx = context_with(src);
    run_op(&mut ctx, "colorkey", "#ff00ff", &[]).unwrap();
    assert!(near(ctx.image.pixel(0, 0)[3], 0.0, 1e-6));
    assert!(near(ctx.image.pixel(1, 0)[3], 1.0, 1e-6));
}

#[test]
fn op_normalmap_flat() {
    let mut ctx = context_with(Image::filled(8, 8, Color::new(0.5, 0.5, 0.5, 1.0)));
    run_op(&mut ctx, "normalmap", "", &[]).unwrap();
    let p = ctx.image.pixel(3, 3);
    assert!(near(p[0], 0.5, 1e-5) && near(p[1], 0.5, 1e-5) && near(p[2], 1.0, 1e-5));
}

#[test]
fn op_unknown_option_rejected() {
    let mut ctx = context_with(make_test_image(4, 4, false));
    assert!(run_op(&mut ctx, "crop", "2x2", &[("gravty", "c")]).unwrap_err().is_usage());
}

#[test]
fn mip_chain_sizes() {
    let chain = build_mip_chain(&make_test_image(256, 64, true), &MipOptions::default()).unwrap();
    assert_eq!(chain.len(), 9);
    assert_eq!(full_mip_count(256, 64), 9);
    let last = chain.last().unwrap();
    assert_eq!((last.width, last.height), (1, 1));
    assert_eq!((chain[3].width, chain[3].height), (32, 8));
}

#[test]
fn alpha_coverage_preserved() {
    // Fine alpha-tested detail (foliage, grates) loses coverage when
    // downsampling averages alpha towards the middle.
    let mut src = Image::filled(64, 64, Color::new(1.0, 1.0, 1.0, 0.0));
    for y in 0..64 {
        for x in 0..64 {
            src.pixel_mut(x, y)[3] = 0.5 + 0.5 * (x as f32 * 1.3).sin() * (y as f32 * 0.9).sin() - 0.15;
        }
    }
    clamp_image(&mut src);
    let target = alpha_coverage(&src, 0.5);
    let kept_opts = MipOptions { coverage_ref: 0.5, ..Default::default() };
    let a = build_mip_chain(&src, &MipOptions::default()).unwrap();
    let b = build_mip_chain(&src, &kept_opts).unwrap();
    for i in 1..4 {
        let drift = (alpha_coverage(&a[i], 0.5) - target).abs();
        let kept = (alpha_coverage(&b[i], 0.5) - target).abs();
        assert!(kept <= drift);
        assert!(kept <= 0.05);
    }
}

/// Operations that produce a new size must refuse results beyond the
/// dimension and pixel limits instead of allocating them.
#[test]
fn size_producing_ops_enforce_limits() {
    let big = || context_with(Image::new(40000, 2));
    let cases: &[(&str, &str, &[(&str, &str)])] = &[
        ("resize", "70000x10", &[]),
        ("resize", "60000x60000", &[]),
        ("resize", "1000%", &[]),
        ("canvas", "65537x10", &[]),
        ("pad", "65536", &[]),
        ("tile", "256", &[]),
        ("tile", "2x1", &[]),
        ("crop", "70000x10", &[("clip", "0")]),
        ("rotate", "45", &[]),
    ];
    for (op, arg, opts) in cases {
        let mut ctx = big();
        let e = run_op(&mut ctx, op, arg, opts).expect_err(&format!("--{op} {arg}"));
        assert!(!e.is_usage(), "--{op} {arg}: {e}");
        assert!(e.message().contains("--") || e.message().contains("size"), "--{op} {arg}: {e}");
        assert_eq!(ctx.image.width, 40000, "--{op} {arg} changed the image");
    }
    // A 16384x16384 image is legal; rotating it by 45 degrees is not.
    let mut ctx = context_with(Image::new(1, 1));
    run_op(&mut ctx, "resize", "16384x16384", &[("filter", "nearest")]).unwrap();
    assert!(run_op(&mut ctx, "rotate", "45", &[]).is_err());
}

#[test]
fn extreme_arguments_are_rejected_cleanly() {
    let mut ctx = context_with(make_test_image(8, 8, true));
    for (op, arg) in [
        ("resize", "0x0"),
        ("resize", "1e300x1"),
        ("crop", "1e300x1+1e300+0"),
        ("pad", "-9223372036854775808"),
        ("pad", "99999999999999999999"),
        ("tile", "0"),
        ("tile", "x"),
        ("rotate", "1e400"),
        ("pan", "1e300,1e300"),
        ("levels", "0,0"),
        ("blur", "1e9"),
        ("swizzle", "rgbä"),
    ] {
        // Any outcome but a panic or a hang is fine; most are errors.
        let _ = run_op(&mut ctx, op, arg, &[]);
        assert!(ctx.image.width >= 1 && ctx.image.height >= 1);
    }
}

#[test]
fn ops_are_deterministic_across_thread_counts() {
    let src = make_test_image(97, 61, true);
    let pipeline = |threads| {
        with_threads(threads, || {
            let mut ctx = context_with(src.clone());
            run_op(&mut ctx, "rotate", "17", &[]).unwrap();
            run_op(&mut ctx, "blur", "1.5", &[]).unwrap();
            run_op(&mut ctx, "sharpen", "0.7", &[]).unwrap();
            run_op(&mut ctx, "normalmap", "", &[]).unwrap();
            run_op(&mut ctx, "hue", "40", &[]).unwrap();
            ctx.image
        })
    };
    assert!(pipeline(1).data == pipeline(7).data);
}
