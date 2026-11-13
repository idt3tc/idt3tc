//! Geometric operations: resize, pot, crop, canvas, pad, trim, rotate,
//! flip, transpose, pan, tile.
//!
//! Sizes derived from user input and the current image are computed in
//! i64 and validated with check_dimensions before anything is allocated.

use super::context::Context;
use super::image::{Color, Image, MAX_DIMENSION, check_dimensions, is_power_of_two};
use super::io_ops::GRAVITY_HELP;
use super::op::{Op, OpInfo, OptionHelp, opt, resample_option_help, round_to_int};
use super::options::OptionReader;
use super::resample::{Filter, resample};
use super::sampling::{EdgeMode, Interp};
use super::spec::{Geometry, Gravity, gravity_origin, parse_geometry, parse_length_pair};
use super::transform::{
    AffineOptions, extract_region, flip_horizontal, flip_vertical, rotate_arbitrary, rotate_quarter, translate,
    transpose,
};
use crate::common::util::{parse_double, parse_long, split};
use crate::common::{Error, Result};

// --- resize -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fit {
    Stretch,
    Contain,
    Cover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Only {
    Always,
    Shrink,
    Enlarge,
}

fn create_resize(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let g = parse_geometry(arg, "--resize")?;
    if g.has_offset {
        return Err(Error::usage("--resize: offsets are not allowed (use --crop or --canvas)"));
    }
    let fit = [Fit::Stretch, Fit::Contain, Fit::Cover][o.choice("fit", &["stretch", "contain", "cover"], 0)?];
    let only = [Only::Always, Only::Shrink, Only::Enlarge][o.choice("only", &["always", "shrink", "enlarge"], 0)?];
    let gravity = o.gravity("gravity", Gravity::Center)?;
    let rs = o.resample_options(Filter::Mitchell, EdgeMode::Clamp, "")?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--resize")?;
        let (sw, sh) = (img.width as f64, img.height as f64);
        let (tw, th) = if g.has_w && g.has_h {
            (g.w.resolve(sw), g.h.resolve(sh))
        } else if g.has_w {
            let tw = g.w.resolve(sw);
            (tw, sh * tw / sw)
        } else {
            let th = g.h.resolve(sh);
            (sw * th / sh, th)
        };

        let mut rw = round_to_int(tw).max(1);
        let mut rh = round_to_int(th).max(1);
        let (mut crop_w, mut crop_h) = (0i64, 0i64);
        if g.has_w && g.has_h && fit != Fit::Stretch {
            let s = if fit == Fit::Contain { (tw / sw).min(th / sh) } else { (tw / sw).max(th / sh) };
            if fit == Fit::Cover {
                crop_w = rw;
                crop_h = rh;
            }
            rw = round_to_int(sw * s).max(1);
            rh = round_to_int(sh * s).max(1);
        }

        let final_w = if crop_w != 0 { crop_w } else { rw };
        let final_h = if crop_h != 0 { crop_h } else { rh };
        let (iw, ih) = (img.width as i64, img.height as i64);
        if only == Only::Shrink && final_w >= iw && final_h >= ih {
            ctx.verbose("--resize: skipped (only=shrink)");
            return Ok(());
        }
        if only == Only::Enlarge && final_w <= iw && final_h <= ih {
            ctx.verbose("--resize: skipped (only=enlarge)");
            return Ok(());
        }

        // The intermediate size of fit=cover may exceed the final one.
        check_dimensions(rw, rh, "--resize")?;
        let mut out = resample(img, rw as i32, rh as i32, &rs)?;
        if crop_w != 0 {
            let (x, y) = gravity_origin(gravity, rw, rh, crop_w, crop_h);
            out = extract_region(&out, x, y, crop_w as i32, crop_h as i32, EdgeMode::Clamp, &Color::TRANSPARENT)?;
        }
        let msg = format!("--resize: {}x{} -> {}x{}", img.width, img.height, out.width, out.height);
        ctx.set_image(out);
        ctx.verbose(msg);
        Ok(())
    }))
}

// --- pot ----------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PotMode {
    Nearest,
    Up,
    Down,
}

fn round_pot(n: i32, mode: PotMode) -> i32 {
    let mut down = 1;
    while down * 2 <= n {
        down *= 2;
    }
    let up = if down == n { n } else { down * 2 };
    match mode {
        PotMode::Down => down,
        PotMode::Up => up,
        // Ties round up.
        PotMode::Nearest => {
            if (n - down) < (up - n) {
                down
            } else {
                up
            }
        }
    }
}

fn create_pot(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    let mode = [PotMode::Nearest, PotMode::Up, PotMode::Down][o.choice("mode", &["nearest", "up", "down"], 0)?];
    let max_size = o.integer("max", 0, 0, MAX_DIMENSION)?;
    let min_size = o.integer("min", 1, 1, MAX_DIMENSION)?;
    if max_size != 0 && !is_power_of_two(max_size as i64) {
        return Err(Error::usage("--pot: max must be a power of two"));
    }
    if !is_power_of_two(min_size as i64) {
        return Err(Error::usage("--pot: min must be a power of two"));
    }
    let square = o.flag("square", false)?;
    let canvas = o.choice("method", &["resample", "canvas"], 0)? == 1;
    let gravity = o.gravity("gravity", Gravity::Center)?;
    let rs = o.resample_options(Filter::Mitchell, if canvas { EdgeMode::Fill } else { EdgeMode::Clamp }, "")?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--pot")?;
        // Dimensions are at most 65536, so the powers of two stay in range.
        let mut w = round_pot(img.width, mode);
        let mut h = round_pot(img.height, mode);
        if square {
            let s = if mode == PotMode::Down { w.min(h) } else { w.max(h) };
            w = s;
            h = s;
        }
        if max_size != 0 {
            w = w.min(max_size);
            h = h.min(max_size);
        }
        w = w.max(min_size);
        h = h.max(min_size);
        if w == img.width && h == img.height {
            return Ok(());
        }
        check_dimensions(w as i64, h as i64, "--pot")?;
        let msg = format!("--pot: {}x{} -> {w}x{h}", img.width, img.height);
        let out = if canvas {
            let (x, y) = gravity_origin(gravity, img.width as i64, img.height as i64, w as i64, h as i64);
            extract_region(img, x, y, w, h, rs.edge, &rs.fill)?
        } else {
            resample(img, w, h, &rs)?
        };
        ctx.verbose(msg);
        ctx.set_image(out);
        Ok(())
    }))
}

// --- crop / canvas ------------------------------------------------------------

/// A region inside (or around) the current image.
struct Region {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Resolves geometry to a region relative to the current image.
fn resolve_region(g: &Geometry, gravity: Gravity, img: &Image) -> Result<Region> {
    let (iw, ih) = (img.width as f64, img.height as f64);
    let w = if g.has_w { round_to_int(g.w.resolve(iw)) } else { img.width as i64 };
    let mut h = if g.has_h { round_to_int(g.h.resolve(ih)) } else { img.height as i64 };
    if g.has_w && !g.has_h && !g.w.percent {
        // "256" alone: width only, full height.
        h = img.height as i64;
    }
    if w < 1 || h < 1 {
        return Err(Error::runtime("region size resolves to less than one pixel"));
    }
    let (mut x, mut y) = gravity_origin(gravity, img.width as i64, img.height as i64, w, h);
    if g.has_offset {
        // round_to_int saturates, so huge offsets cannot overflow here.
        x = x.saturating_add(round_to_int(g.x.resolve(iw)));
        y = y.saturating_add(round_to_int(g.y.resolve(ih)));
    }
    Ok(Region { x, y, w, h })
}

fn create_crop(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let g = parse_geometry(arg, "--crop")?;
    let gravity = o.gravity("gravity", Gravity::NorthWest)?;
    let clip = o.flag("clip", true)?;
    let edge = o.edge("edge", EdgeMode::Fill)?;
    let bg = o.color("bg", Color::TRANSPARENT)?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--crop")?;
        let Region { mut x, mut y, mut w, mut h } = resolve_region(&g, gravity, img)?;
        if clip {
            let (x0, y0) = (x.max(0), y.max(0));
            let x1 = (img.width as i64).min(x.saturating_add(w));
            let y1 = (img.height as i64).min(y.saturating_add(h));
            if x1 <= x0 || y1 <= y0 {
                return Err(Error::runtime(format!(
                    "--crop: region {w}x{h}{x:+}{y:+} lies outside the {}x{} image",
                    img.width, img.height
                )));
            }
            (x, y, w, h) = (x0, y0, x1 - x0, y1 - y0);
        }
        check_dimensions(w, h, "--crop")?;
        let out = extract_region(img, x, y, w as i32, h as i32, edge, &bg)?;
        ctx.verbose(format!("--crop: {w}x{h}{x:+}{y:+}"));
        ctx.set_image(out);
        Ok(())
    }))
}

fn create_canvas(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let g = parse_geometry(arg, "--canvas")?;
    let gravity = o.gravity("gravity", Gravity::Center)?;
    let edge = o.edge("edge", EdgeMode::Fill)?;
    let bg = o.color("bg", Color::TRANSPARENT)?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--canvas")?;
        let r = resolve_region(&g, gravity, img)?;
        check_dimensions(r.w, r.h, "--canvas")?;
        let out = extract_region(img, r.x, r.y, r.w as i32, r.h as i32, edge, &bg)?;
        ctx.set_image(out);
        Ok(())
    }))
}

// --- pad ----------------------------------------------------------------------

fn create_pad(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let parts = split(arg, ',');
    let mut v = [0i64; 4];
    for (d, p) in v.iter_mut().zip(&parts) {
        *d = parse_long(p)
            .filter(|n| (-(MAX_DIMENSION as i64)..=MAX_DIMENSION as i64).contains(n))
            .ok_or_else(|| Error::usage(format!("--pad: invalid value '{p}'")))?;
    }
    let (left, top, right, bottom) = match parts.len() {
        1 => (v[0], v[0], v[0], v[0]),
        2 => (v[0], v[1], v[0], v[1]),
        4 => (v[0], v[1], v[2], v[3]),
        _ => return Err(Error::usage("--pad: expected N, H,V or L,T,R,B")),
    };
    let edge = o.edge("edge", EdgeMode::Fill)?;
    let bg = o.color("bg", Color::TRANSPARENT)?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--pad")?;
        let w = img.width as i64 + left + right;
        let h = img.height as i64 + top + bottom;
        if w < 1 || h < 1 {
            return Err(Error::runtime("--pad: negative padding removes the whole image"));
        }
        check_dimensions(w, h, "--pad")?;
        let out = extract_region(img, -left, -top, w as i32, h as i32, edge, &bg)?;
        ctx.set_image(out);
        Ok(())
    }))
}

// --- trim ---------------------------------------------------------------------

fn create_trim(_arg: &str, o: &mut OptionReader) -> Result<Op> {
    let mode = o.choice("mode", &["auto", "alpha", "color"], 0)?;
    let has_color = o.has("color");
    let color = o.color("color", Color::TRANSPARENT)?;
    let fuzz = o.number("fuzz", 0.0, 0.0, 1.0)? as f32;
    let margin = o.integer("margin", 0, 0, MAX_DIMENSION)?;
    if has_color && mode == 1 {
        return Err(Error::usage("--trim: color= cannot be combined with mode=alpha"));
    }

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--trim")?;
        // Decide what counts as "border": transparent pixels or pixels
        // matching a color (explicit or taken from the top-left corner).
        let mut by_alpha = mode == 1;
        let mut reference = color.to_array();
        if mode == 0 && !has_color {
            let corner = img.pixel(0, 0);
            by_alpha = corner[3] <= fuzz;
            reference = [corner[0], corner[1], corner[2], corner[3]];
        }
        let is_border = |p: &[f32]| {
            if by_alpha {
                p[3] <= fuzz
            } else {
                (0..4).all(|c| (p[c] - reference[c]).abs() <= fuzz + 1e-6)
            }
        };
        let (mut x0, mut y0, mut x1, mut y1) = (img.width, img.height, -1, -1);
        for y in 0..img.height {
            for (x, p) in img.row(y).chunks_exact(4).enumerate() {
                if !is_border(p) {
                    let x = x as i32;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        if x1 < 0 {
            ctx.info("--trim: image consists only of border pixels, left unchanged");
            return Ok(());
        }
        x0 = (x0 - margin).max(0);
        y0 = (y0 - margin).max(0);
        x1 = (x1 + margin).min(img.width - 1);
        y1 = (y1 + margin).min(img.height - 1);
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let out = extract_region(img, x0 as i64, y0 as i64, w, h, EdgeMode::Clamp, &Color::TRANSPARENT)?;
        ctx.verbose(format!("--trim: {w}x{h}+{x0}+{y0}"));
        ctx.set_image(out);
        Ok(())
    }))
}

// --- rotate -------------------------------------------------------------------

fn affine_options(o: &mut OptionReader, def_interp: Interp, def_edge: EdgeMode) -> Result<AffineOptions> {
    Ok(AffineOptions {
        interp: o.interp("filter", def_interp)?,
        edge: o.edge("edge", def_edge)?,
        fill: o.color("bg", Color::TRANSPARENT)?,
        premultiply: o.flag("premultiply", true)?,
    })
}

fn affine_help(def_interp: &str, def_edge: &str) -> Vec<OptionHelp> {
    vec![
        opt("filter", def_interp, "interpolation: nearest, bilinear, bicubic"),
        opt("edge", def_edge, "source outside the image: fill, clamp, repeat, mirror"),
        opt("bg", "transparent", "color used with edge=fill"),
        opt("premultiply", "1", "interpolate alpha-weighted colors"),
    ]
}

fn create_rotate(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let deg = parse_double(arg).ok_or_else(|| Error::usage(format!("--rotate: invalid angle '{arg}'")))?;
    let expand = o.flag("expand", true)?;
    let a = affine_options(o, Interp::Bicubic, EdgeMode::Fill)?;

    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--rotate")?;
        // Rust's % on floats is C's fmod().
        let mut norm = deg % 360.0;
        if norm < 0.0 {
            norm += 360.0;
        }
        let q = norm / 90.0;
        let quarters = round_to_int(q);
        let exact_quarter = (q - quarters as f64).abs() < 1e-9;
        // Quarter turns are lossless unless a non-square image must keep
        // its canvas (expand=0 with 90/270 degrees).
        let out = if exact_quarter && (expand || quarters % 2 == 0 || img.width == img.height) {
            rotate_quarter(img, quarters as i32)
        } else {
            rotate_arbitrary(img, norm, expand, &a)?
        };
        ctx.set_image(out);
        Ok(())
    }))
}

// --- flip / transpose ---------------------------------------------------------

fn create_flip(arg: &str, _o: &mut OptionReader) -> Result<Op> {
    let (h, v) = match arg.to_ascii_lowercase().as_str() {
        "h" | "x" | "horizontal" => (true, false),
        "v" | "y" | "vertical" => (false, true),
        "hv" | "vh" | "xy" | "both" => (true, true),
        _ => return Err(Error::usage(format!("--flip: expected h, v or hv, got '{arg}'"))),
    };
    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--flip")?;
        let mut out = if h { flip_horizontal(img) } else { img.clone() };
        if v {
            out = flip_vertical(&out);
        }
        ctx.set_image(out);
        Ok(())
    }))
}

fn create_transpose(_arg: &str, _o: &mut OptionReader) -> Result<Op> {
    Ok(Op::new(|ctx: &mut Context| {
        let out = transpose(ctx.require_image("--transpose")?);
        ctx.set_image(out);
        Ok(())
    }))
}

// --- pan ----------------------------------------------------------------------

fn create_pan(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let (lx, ly) = parse_length_pair(arg, "--pan")?;
    let a = affine_options(o, Interp::Bilinear, EdgeMode::Repeat)?;
    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--pan")?;
        let out = translate(img, lx.resolve(img.width as f64), ly.resolve(img.height as f64), &a);
        ctx.set_image(out);
        Ok(())
    }))
}

// --- tile ---------------------------------------------------------------------

fn create_tile(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let a = arg.to_ascii_lowercase();
    let bad = || Error::usage(format!("--tile: expected N or NxM, got '{arg}'"));
    let (nx, ny) = match a.find('x') {
        None => {
            let n = parse_long(&a).ok_or_else(bad)?;
            (n, n)
        }
        Some(p) => (parse_long(&a[..p]).ok_or_else(bad)?, parse_long(&a[p + 1..]).ok_or_else(bad)?),
    };
    if !(1..=256).contains(&nx) || !(1..=256).contains(&ny) {
        return Err(Error::usage("--tile: counts must be in 1..256"));
    }
    let mirror = o.flag("mirror", false)?;
    Ok(Op::new(move |ctx| {
        let img = ctx.require_image("--tile")?;
        let (w, h) = (img.width as i64 * nx, img.height as i64 * ny);
        check_dimensions(w, h, "--tile")?;
        let edge = if mirror { EdgeMode::Mirror } else { EdgeMode::Repeat };
        let out = extract_region(img, 0, 0, w as i32, h as i32, edge, &Color::TRANSPARENT)?;
        ctx.set_image(out);
        Ok(())
    }))
}

fn with(mut base: Vec<OptionHelp>, extra: Vec<OptionHelp>) -> Vec<OptionHelp> {
    base.extend(extra);
    base
}

pub fn ops() -> Vec<OpInfo> {
    let info = |name, aliases, arg, summary: &str, details: &str, options, create| OpInfo {
        name,
        aliases,
        arg,
        category: "Geometry",
        summary: summary.to_string(),
        details: details.to_string(),
        options,
        create,
    };
    vec![
        info(
            "resize",
            &["scale"],
            "GEOMETRY",
            "resample to a new size",
            "GEOMETRY is WxH, Wx or W (keep aspect, compute height), xH (compute width), N% or W%xH%. With both W \
             and H given, fit= decides how the aspect ratio is handled.",
            with(
                vec![
                    opt(
                        "fit",
                        "stretch",
                        "stretch: exactly WxH; contain: largest size fitting into WxH; cover: fill WxH, crop the \
                         overflow",
                    ),
                    opt("only", "always", "always, shrink (never enlarge) or enlarge (never shrink)"),
                    opt("gravity", "center", &format!("crop anchor for fit=cover; {GRAVITY_HELP}")),
                ],
                resample_option_help("mitchell", "clamp", ""),
            ),
            create_resize,
        ),
        info(
            "pot",
            &[],
            "",
            "make both dimensions powers of two (required by idTech 3 for undistorted textures)",
            "Rounds each dimension to a power of two, then resamples (method=resample) or pads/crops the canvas \
             (method=canvas).",
            with(
                vec![
                    opt("mode", "nearest", "rounding: nearest (ties go up), up, down"),
                    opt(
                        "max",
                        "0",
                        "upper limit per dimension (power of two), 0 = none; e.g. 2048 to stay within the texture \
                         size limits of older engines/drivers",
                    ),
                    opt("min", "1", "lower limit per dimension (power of two)"),
                    opt("square", "0", "force a square result"),
                    opt("method", "resample", "resample: scale the image; canvas: pad/crop without scaling"),
                    opt("gravity", "center", &format!("method=canvas anchor; {GRAVITY_HELP}")),
                ],
                resample_option_help("mitchell", "clamp (fill for method=canvas)", ""),
            ),
            create_pot,
        ),
        info(
            "crop",
            &[],
            "GEOMETRY",
            "cut out a region",
            "GEOMETRY is WxH+X+Y (also WxH, +X+Y, percentages). The offset is relative to the gravity anchor, e.g. \
             '256x256 gravity=center' takes the middle.",
            vec![
                opt("gravity", "nw", GRAVITY_HELP),
                opt("clip", "1", "clip the region to the image; with clip=0 outside parts use edge/bg"),
                opt("edge", "fill", "with clip=0: fill, clamp, repeat, mirror"),
                opt("bg", "transparent", "fill color for edge=fill"),
            ],
            create_crop,
        ),
        info(
            "canvas",
            &["extent"],
            "GEOMETRY",
            "change the canvas size without scaling (pad or crop)",
            "GEOMETRY is WxH[+X+Y]; the image is placed by gravity, new areas are filled per edge=.",
            vec![
                opt("gravity", "center", GRAVITY_HELP),
                opt("edge", "fill", "new areas: fill (bg color), clamp, repeat, mirror"),
                opt("bg", "transparent", "fill color"),
            ],
            create_canvas,
        ),
        info(
            "pad",
            &["border"],
            "N|H,V|L,T,R,B",
            "add (or with negative values remove) pixels at the borders",
            "",
            vec![
                opt("edge", "fill", "new areas: fill (bg color), clamp, repeat, mirror"),
                opt("bg", "transparent", "fill color"),
            ],
            create_pad,
        ),
        info(
            "trim",
            &[],
            "",
            "remove uniform or transparent borders",
            "",
            vec![
                opt("mode", "auto", "auto (alpha if the top-left pixel is transparent, else its color), alpha, color"),
                opt("color", "", "border color (implies mode=color semantics)"),
                opt("fuzz", "0", "tolerance per channel (0..1)"),
                opt("margin", "0", "keep this many pixels around the content"),
            ],
            create_trim,
        ),
        info(
            "rotate",
            &[],
            "DEGREES",
            "rotate clockwise (multiples of 90 are lossless)",
            "",
            with(vec![opt("expand", "1", "grow the canvas to fit the rotated image")], affine_help("bicubic", "fill")),
            create_rotate,
        ),
        info("flip", &["mirror"], "h|v|hv", "mirror horizontally (h), vertically (v) or both", "", vec![], create_flip),
        info(
            "transpose",
            &[],
            "",
            "swap rows and columns (mirror along the main diagonal)",
            "",
            vec![],
            create_transpose,
        ),
        info(
            "pan",
            &["offset", "shift", "roll"],
            "DX,DY",
            "shift the content; wraps around by default (seam checking)",
            "DX,DY in pixels (fractional allowed) or percent, e.g. 50%,50% moves the texture seams to the center. \
             Positive values move right/down.",
            affine_help("bilinear", "repeat"),
            create_pan,
        ),
        info(
            "tile",
            &[],
            "N|NxM",
            "repeat the image N x M times",
            "",
            vec![opt("mirror", "0", "mirror every other copy")],
            create_tile,
        ),
    ]
}
