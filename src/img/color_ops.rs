//! Per-pixel color, alpha and filter operations.
//!
//! Every pixel is processed independently, so the loops run in parallel
//! on chunks of the image; the result does not depend on the thread count.

use rayon::prelude::*;

use super::image::{Color, Image, clamp01, luma, premultiply, unpremultiply};
use super::mipmap::scale_alpha_to_coverage;
use super::op::{Op, OpFactory, OpInfo, OptionHelp, opt};
use super::options::OptionReader;
use super::resample::gaussian_blur;
use super::sampling::{EdgeMode, resolve_edge};
use super::spec::parse_color;
use crate::common::util::{parse_double, split, to_lower};
use crate::common::{Error, Result};

const CHANNELS_HELP: &str = "channels to modify, any of r, g, b, a";

/// Floats per parallel work item: big enough to amortize scheduling,
/// small enough to spread medium sized images over all threads.
const CHUNK: usize = 4 * 16384;

fn parse_number_arg(op: &str, arg: &str) -> Result<f64> {
    parse_double(arg).ok_or_else(|| Error::usage(format!("{op}: expected a number, got '{arg}'")))
}

fn parse_color_arg(op: &str, arg: &str) -> Result<Color> {
    parse_color(arg).ok_or_else(|| Error::usage(format!("{op}: invalid color '{arg}'")))
}

/// Runs `f` on every RGBA pixel, in parallel.
fn for_each_pixel(img: &mut Image, f: impl Fn(&mut [f32]) + Sync) {
    img.data.par_chunks_mut(CHUNK).for_each(|chunk| chunk.chunks_exact_mut(4).for_each(&f));
}

/// Ops that map each selected channel independently through `f`; the
/// result is clamped to 0..1.
fn channel_op(name: &'static str, o: &mut OptionReader, f: impl Fn(f32) -> f32 + Send + Sync + 'static) -> Result<Op> {
    let mask = o.channels("channels", 7)?;
    Ok(Op::new(move |ctx| {
        let img = ctx.require_image(name)?;
        for_each_pixel(img, |p| {
            for (c, v) in p.iter_mut().enumerate() {
                if mask & (1 << c) != 0 {
                    *v = clamp01(f(*v));
                }
            }
        });
        Ok(())
    }))
}

fn create_gamma(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let g = parse_number_arg("--gamma", arg)?;
    if g <= 0.0 {
        return Err(Error::usage("--gamma: value must be > 0"));
    }
    let inv = (1.0 / g) as f32;
    channel_op("--gamma", o, move |v| if v <= 0.0 { 0.0 } else { libm::powf(v, inv) })
}

fn create_brightness(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let d = parse_number_arg("--brightness", arg)? as f32;
    channel_op("--brightness", o, move |v| v + d)
}

fn create_contrast(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let f = parse_number_arg("--contrast", arg)? as f32;
    let pivot = o.number("pivot", 0.5, 0.0, 1.0)? as f32;
    channel_op("--contrast", o, move |v| (v - pivot) * f + pivot)
}

fn create_levels(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let parts = split(arg, ',');
    let mut v = [0.0, 255.0, 1.0, 0.0, 255.0];
    if ![2, 3, 5].contains(&parts.len()) {
        return Err(Error::usage("--levels: expected IN_LO,IN_HI[,GAMMA[,OUT_LO,OUT_HI]]"));
    }
    for (d, p) in v.iter_mut().zip(&parts) {
        *d = parse_double(p).ok_or_else(|| Error::usage(format!("--levels: invalid number '{p}'")))?;
    }
    // Written so that NaN fails the check as well.
    if !(v[1] > v[0] && v[2] > 0.0) {
        return Err(Error::usage("--levels: need IN_LO < IN_HI and GAMMA > 0"));
    }
    let (in_lo, in_hi) = ((v[0] / 255.0) as f32, (v[1] / 255.0) as f32);
    let inv_gamma = (1.0 / v[2]) as f32;
    let (out_lo, out_hi) = ((v[3] / 255.0) as f32, (v[4] / 255.0) as f32);
    channel_op("--levels", o, move |c| {
        let t = clamp01((c - in_lo) / (in_hi - in_lo));
        out_lo + libm::powf(t, inv_gamma) * (out_hi - out_lo)
    })
}

fn create_invert(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    channel_op("--invert", o, |v| 1.0 - v)
}

fn create_saturation(arg: &str, _o: &mut OptionReader) -> Result<Op> {
    let f = parse_number_arg("--saturation", arg)? as f32;
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--saturation")?, |p| {
            let l = luma(p);
            for v in &mut p[..3] {
                *v = clamp01(l + (*v - l) * f);
            }
        });
        Ok(())
    }))
}

fn create_hue(arg: &str, _o: &mut OptionReader) -> Result<Op> {
    let deg = parse_number_arg("--hue", arg)?;
    // Rotation around the gray axis (1,1,1) of the RGB cube.
    let a = deg * std::f64::consts::PI / 180.0;
    let (cs, sn) = (libm::cos(a) as f32, libm::sin(a) as f32);
    let k = (1.0 - cs) / 3.0;
    let s3 = (1.0f32 / 3.0).sqrt() * sn;
    let m = [[cs + k, k - s3, k + s3], [k + s3, cs + k, k - s3], [k - s3, k + s3, cs + k]];
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--hue")?, |p| {
            let (r, g, b) = (p[0], p[1], p[2]);
            for (v, row) in p.iter_mut().zip(&m) {
                *v = clamp01(row[0] * r + row[1] * g + row[2] * b);
            }
        });
        Ok(())
    }))
}

fn create_grayscale(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    let w = o.choice("weights", &["rec709", "rec601", "average", "max"], 0)?;
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--grayscale")?, |p| {
            let v = match w {
                0 => luma(p),
                1 => 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2],
                2 => (p[0] + p[1] + p[2]) / 3.0,
                _ => p[0].max(p[1].max(p[2])),
            };
            let v = clamp01(v);
            p[..3].fill(v);
        });
        Ok(())
    }))
}

fn create_tint(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let c = parse_color_arg("--tint", arg)?.to_array();
    let mask = o.channels("channels", 7)?;
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--tint")?, |p| {
            for ch in 0..4 {
                if mask & (1 << ch) != 0 {
                    p[ch] *= c[ch];
                }
            }
        });
        Ok(())
    }))
}

fn create_fill(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let c = parse_color_arg("--fill", arg)?.to_array();
    let mask = o.channels("channels", 15)?;
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--fill")?, |p| {
            for ch in 0..4 {
                if mask & (1 << ch) != 0 {
                    p[ch] = c[ch];
                }
            }
        });
        Ok(())
    }))
}

fn create_colorkey(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let key = parse_color_arg("--colorkey", arg)?;
    let fuzz = o.number("fuzz", 0.0, 0.0, 1.0)? as f32;
    let soft = o.number("soft", 0.0, 0.0, 1.0)? as f32;
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--colorkey")?, |p| {
            let d = (p[0] - key.r).abs().max((p[1] - key.g).abs().max((p[2] - key.b).abs()));
            if d <= fuzz + 1e-6 {
                p[3] = 0.0;
            } else if soft > 0.0 && d < fuzz + soft {
                p[3] *= (d - fuzz) / soft;
            }
        });
        Ok(())
    }))
}

const ALPHA_MODES: &[&str] =
    &["opaque", "clear", "set", "scale", "invert", "luma", "threshold", "premultiply", "unpremultiply", "coverage"];

fn create_alpha(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let m = to_lower(arg);
    let mode = ALPHA_MODES.iter().position(|&x| x == m).ok_or_else(|| {
        Error::usage(format!("--alpha: unknown mode '{arg}' (expected {})", ALPHA_MODES.join(", ")))
    })?;

    let (mut value, mut reference, mut target) = (1.0f32, 0.5f32, -1.0f32);
    if m == "set" || m == "scale" {
        if !o.has("value") {
            return Err(Error::usage(format!("--alpha {m}: needs value=...")));
        }
        value = o.number("value", 1.0, 0.0, if m == "set" { 1.0 } else { 100.0 })? as f32;
    }
    if m == "threshold" || m == "coverage" {
        reference = o.number("ref", 0.5, 0.0, 1.0)? as f32;
    }
    if m == "coverage" {
        target = o.number("target", 0.5, 0.0, 1.0)? as f32;
    }

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--alpha")?;
        match mode {
            7 => premultiply(img),
            8 => unpremultiply(img),
            9 => scale_alpha_to_coverage(img, reference, target),
            _ => for_each_pixel(img, |p| {
                p[3] = match mode {
                    0 => 1.0,
                    1 => 0.0,
                    2 => value,
                    3 => clamp01(p[3] * value),
                    4 => 1.0 - p[3],
                    5 => clamp01(luma(p)),
                    _ => {
                        if p[3] >= reference {
                            1.0
                        } else {
                            0.0
                        }
                    }
                };
            }),
        }
        Ok(())
    }))
}

fn create_flatten(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    let bg = o.color("bg", Color::BLACK)?.to_array();
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--flatten")?, |p| {
            let a = p[3];
            for c in 0..3 {
                p[c] = p[c] * a + bg[c] * (1.0 - a);
            }
            p[3] = 1.0;
        });
        Ok(())
    }))
}

fn create_swizzle(arg: &str, _o: &mut OptionReader) -> Result<Op> {
    let mut s = to_lower(arg).into_bytes();
    if s.len() == 3 {
        s.push(b'a');
    }
    if s.len() != 4 {
        return Err(Error::usage("--swizzle: expected 3 or 4 characters from r, g, b, a, 0, 1 (e.g. bgra, rrr1)"));
    }
    // Source index per destination channel; 4 = constant 0, 5 = constant 1.
    let mut src = [0usize; 4];
    for (d, &ch) in src.iter_mut().zip(&s) {
        *d = b"rgba01".iter().position(|&x| x == ch).ok_or_else(|| {
            Error::usage(format!("--swizzle: invalid channel '{}' in '{arg}'", char::from(ch)))
        })?;
    }
    Ok(Op::new(move |ctx| {
        for_each_pixel(ctx.require_image("--swizzle")?, |p| {
            let inp = [p[0], p[1], p[2], p[3], 0.0, 1.0];
            for (v, &i) in p.iter_mut().zip(&src) {
                *v = inp[i];
            }
        });
        Ok(())
    }))
}

fn create_blur(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let sigma = parse_number_arg("--blur", arg)?;
    // Written so that NaN is rejected too.
    if !(0.0..=500.0).contains(&sigma) {
        return Err(Error::usage("--blur: sigma must be in [0, 500]"));
    }
    let edge = o.edge("edge", EdgeMode::Clamp)?;
    let bg = o.color("bg", Color::TRANSPARENT)?;
    let premul = o.flag("premultiply", true)?;
    Ok(Op::new(move |ctx| {
        let out = gaussian_blur(ctx.require_image("--blur")?, sigma as f32, edge, &bg, premul);
        ctx.set_image(out);
        Ok(())
    }))
}

fn create_sharpen(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let amount = parse_number_arg("--sharpen", arg)? as f32;
    let sigma = o.number("sigma", 1.0, 0.1, 100.0)? as f32;
    let threshold = o.number("threshold", 0.0, 0.0, 1.0)? as f32;
    let edge = o.edge("edge", EdgeMode::Clamp)?;
    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--sharpen")?;
        // Unsharp mask on RGB; alpha is left untouched.
        let blurred = gaussian_blur(img, sigma, edge, &Color::TRANSPARENT, true);
        img.data.par_chunks_mut(CHUNK).zip(blurred.data.par_chunks(CHUNK)).for_each(|(dst, blur)| {
            for (p, b) in dst.chunks_exact_mut(4).zip(blur.chunks_exact(4)) {
                for c in 0..3 {
                    let diff = p[c] - b[c];
                    if diff.abs() > threshold {
                        p[c] = clamp01(p[c] + amount * diff);
                    }
                }
            }
        });
        Ok(())
    }))
}

fn create_normalmap(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    let strength = o.number("strength", 2.0, 0.0, 1000.0)? as f32;
    let source = o.choice("source", &["luma", "r", "g", "b", "a"], 0)?;
    let edge = o.edge("edge", EdgeMode::Repeat)?;
    let flip_x = o.flag("flip-x", false)?;
    let flip_y = o.flag("flip-y", false)?;
    let height_alpha = o.flag("height-alpha", false)?;
    // There is no fill value for heights; edge=fill behaves like clamp.
    let edge = if edge == EdgeMode::Fill { EdgeMode::Clamp } else { edge };
    let sx = if flip_x { -1.0f32 } else { 1.0 };
    let sy = if flip_y { -1.0f32 } else { 1.0 };

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--normalmap")?;
        let (w, h) = (img.width, img.height);
        let height: Vec<f32> =
            img.data.chunks_exact(4).map(|p| if source == 0 { luma(p) } else { p[source - 1] }).collect();
        // resolve_edge never returns None for the modes left at this point.
        let hv = |x: i32, y: i32| {
            let px = resolve_edge(x as i64, w, edge).unwrap_or(0) as usize;
            let py = resolve_edge(y as i64, h, edge).unwrap_or(0) as usize;
            height[py * w as usize + px]
        };
        let mut out = Image::new(w, h);
        let stride = out.stride();
        out.data.par_chunks_mut(stride).enumerate().for_each(|(y, row)| {
            let y = y as i32;
            for (x, d) in row.chunks_exact_mut(4).enumerate() {
                let x = x as i32;
                // Sobel gradients (image y axis points down).
                let dx = (hv(x + 1, y - 1) + 2.0 * hv(x + 1, y) + hv(x + 1, y + 1))
                    - (hv(x - 1, y - 1) + 2.0 * hv(x - 1, y) + hv(x - 1, y + 1));
                let dy = (hv(x - 1, y + 1) + 2.0 * hv(x, y + 1) + hv(x + 1, y + 1))
                    - (hv(x - 1, y - 1) + 2.0 * hv(x, y - 1) + hv(x + 1, y - 1));
                // OpenGL convention: +Y (green) points up in the texture.
                let nx = -dx * strength / 8.0 * sx;
                let ny = dy * strength / 8.0 * sy;
                let nz = 1.0f32;
                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                d[0] = nx / len * 0.5 + 0.5;
                d[1] = ny / len * 0.5 + 0.5;
                d[2] = nz / len * 0.5 + 0.5;
                d[3] = if height_alpha { height[y as usize * w as usize + x as usize] } else { 1.0 };
            }
        });
        ctx.set_image(out);
        Ok(())
    }))
}

pub fn ops() -> Vec<OpInfo> {
    let rgb_channels = || vec![opt("channels", "rgb", CHANNELS_HELP)];
    let op = |name, arg, category, summary: &str, options, create: OpFactory| OpInfo {
        name,
        aliases: &[],
        arg,
        category,
        summary: summary.to_string(),
        details: String::new(),
        options,
        create,
    };
    let cat = "Color & alpha";
    let fcat = "Filters";
    let mut contrast_opts = rgb_channels();
    contrast_opts.push(opt("pivot", "0.5", "value that stays unchanged"));
    let none = Vec::<OptionHelp>::new;

    vec![
        op("gamma", "G", cat, "gamma correction: v = v^(1/G); G > 1 brightens midtones", rgb_channels(), create_gamma),
        op("brightness", "DELTA", cat, "add DELTA (-1..1) to every value", rgb_channels(), create_brightness),
        op(
            "contrast",
            "FACTOR",
            cat,
            "scale values around the pivot; FACTOR > 1 increases contrast",
            contrast_opts,
            create_contrast,
        ),
        op(
            "levels",
            "LO,HI[,GAMMA[,OUTLO,OUTHI]]",
            cat,
            "map input range LO..HI (0-255) to OUTLO..OUTHI with optional gamma",
            rgb_channels(),
            create_levels,
        ),
        op("saturation", "FACTOR", cat, "0 = grayscale, 1 = unchanged, >1 more saturated", none(), create_saturation),
        op("hue", "DEGREES", cat, "rotate hues", none(), create_hue),
        op(
            "grayscale",
            "",
            cat,
            "convert to gray (keeps RGB layout)",
            vec![opt("weights", "rec709", "rec709, rec601, average or max")],
            create_grayscale,
        ),
        op("invert", "", cat, "negate values", rgb_channels(), create_invert),
        op("tint", "COLOR", cat, "multiply by a color", rgb_channels(), create_tint),
        op(
            "fill",
            "COLOR",
            cat,
            "set channels to a constant color",
            vec![opt("channels", "rgba", CHANNELS_HELP)],
            create_fill,
        ),
        op(
            "colorkey",
            "COLOR",
            cat,
            "make pixels of COLOR transparent (e.g. #ff00ff sprite backgrounds)",
            vec![
                opt("fuzz", "0", "max. per-channel difference still matched (0..1)"),
                opt("soft", "0", "feather: alpha ramps up over this extra distance"),
            ],
            create_colorkey,
        ),
        op(
            "alpha",
            "MODE",
            cat,
            "alpha channel ops: opaque, clear, set, scale, invert, luma, threshold, premultiply, unpremultiply, \
             coverage",
            vec![
                opt("value", "", "set: new alpha (0..1); scale: factor"),
                opt("ref", "0.5", "threshold/coverage: alpha reference (0.5 = idTech 3 alphaFunc GE128)"),
                opt("target", "0.5", "coverage: fraction of pixels that should pass the alpha test"),
            ],
            create_alpha,
        ),
        op(
            "flatten",
            "",
            cat,
            "composite onto a solid background and drop alpha",
            vec![opt("bg", "black", "background color")],
            create_flatten,
        ),
        op(
            "swizzle",
            "PATTERN",
            cat,
            "reorder channels, e.g. bgra, rrr1, ggga (r, g, b, a, 0, 1; 3 chars keep alpha)",
            none(),
            create_swizzle,
        ),
        op(
            "blur",
            "SIGMA",
            fcat,
            "Gaussian blur with standard deviation SIGMA pixels",
            vec![
                opt("edge", "clamp", "clamp, repeat (tiling textures), mirror, fill"),
                opt("bg", "transparent", "color for edge=fill"),
                opt("premultiply", "1", "alpha-weighted blur"),
            ],
            create_blur,
        ),
        op(
            "sharpen",
            "AMOUNT",
            fcat,
            "unsharp mask; AMOUNT around 0.5..2",
            vec![
                opt("sigma", "1", "blur radius of the mask"),
                opt("threshold", "0", "minimum difference to sharpen"),
                opt("edge", "clamp", "clamp, repeat, mirror"),
            ],
            create_sharpen,
        ),
        op(
            "normalmap",
            "",
            fcat,
            "turn a height map into a tangent-space normal map",
            vec![
                opt("strength", "2", "height scale (bumpiness)"),
                opt("source", "luma", "height from luma, r, g, b or a"),
                opt("edge", "repeat", "border handling: repeat (tiling), clamp, mirror"),
                opt("flip-x", "0", "invert the red channel"),
                opt("flip-y", "0", "invert green (default is OpenGL style +Y up; flip-y=1 for DirectX style)"),
                opt("height-alpha", "0", "store the height in alpha (for parallax mapping)"),
            ],
            create_normalmap,
        ),
    ]
}
