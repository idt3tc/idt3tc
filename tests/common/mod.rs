//! Shared test fixtures: synthetic images, comparison metrics and helpers
//! for running operations like the command line does.
#![allow(dead_code)]

use idt3tc::common::Result;
use idt3tc::img::context::Context;
use idt3tc::img::image::{Image, to_byte};
use idt3tc::img::op::OpRegistry;
use idt3tc::img::options::OptionReader;

/// Smooth color gradients with a soft alpha ramp and some detail, so block
/// compressors have realistic content to work with.
pub fn make_test_image(w: i32, h: i32, with_alpha: bool) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = img.pixel_mut(x, y);
            let u = (x as f32 + 0.5) / w as f32;
            let v = (y as f32 + 0.5) / h as f32;
            p[0] = u;
            p[1] = v;
            p[2] = 0.5 + 0.5 * (6.2831853f32 * (u + v)).sin();
            p[3] = if with_alpha { 0.25 + 0.75 * u * v } else { 1.0 };
        }
    }
    img
}

/// Peak signal-to-noise ratio (dB) over the channels in `mask` (bit 0 = r)
/// after 8-bit quantization of both images.
pub fn psnr(a: &Image, b: &Image, mask: u32) -> f64 {
    let (mut se, mut n) = (0.0f64, 0usize);
    for (pa, pb) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        for c in 0..4 {
            if mask & (1 << c) != 0 {
                let d = to_byte(pa[c]) as f64 - to_byte(pb[c]) as f64;
                se += d * d;
                n += 1;
            }
        }
    }
    if se == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 / (se / n as f64)).log10() }
}

/// Largest per-channel difference in 8-bit steps.
pub fn max_byte_diff(a: &Image, b: &Image) -> i32 {
    assert_eq!(a.data.len(), b.data.len(), "image sizes differ");
    a.data.iter().zip(&b.data).map(|(x, y)| (to_byte(*x) as i32 - to_byte(*y) as i32).abs()).max().unwrap_or(0)
}

/// Owned key=value list as the option parser takes it.
pub fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Runs a single registered operation like the CLI would.
pub fn run_op(ctx: &mut Context, name: &str, arg: &str, options: &[(&str, &str)]) -> Result<()> {
    let info = OpRegistry::instance().find(name).unwrap_or_else(|| panic!("no op {name}"));
    let mut r = OptionReader::new(&format!("--{name}"), &kv(options))?;
    let op = (info.create)(arg, &mut r)?;
    r.finish()?;
    op.apply(ctx)
}

pub fn context_with(img: Image) -> Context {
    let mut ctx = Context::new(1);
    ctx.set_image(img);
    ctx
}

/// Runs `f` on a dedicated rayon pool with `threads` threads.
pub fn with_threads<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(f)
}
