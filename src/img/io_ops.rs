//! Operations that create, read or write images: new, load, save, info,
//! channel-from, composite.

use super::context::Context;
use super::image::{
    Color, Image, MAX_DIMENSION, analyze_alpha, check_dimensions, clamp01, is_grayscale, is_power_of_two, luma,
};
use super::io::{encode_bmp, encode_jpg, encode_png, load_image};
use super::io_dds::{DdsOptions, dds_codec_name, dds_codec_names, encode_dds, parse_dds_codec};
use super::io_tga::{TgaOptions, encode_tga};
use super::op::{Op, OpInfo, OptionHelp, opt, resample_option_help, round_to_int};
use super::options::OptionReader;
use super::resample::{Filter, resample};
use super::sampling::EdgeMode;
use super::spec::{Gravity, gravity_origin, parse_geometry};
use crate::common::fs;
use crate::common::util::{parse_double, parse_long, split};
use crate::common::{Error, Result};

pub const GRAVITY_HELP: &str = "anchor: nw, n, ne, w, center, e, sw, s, se";

fn size_string(bytes: usize) -> String {
    if bytes < 10 * 1024 {
        format!("{bytes} B")
    } else if bytes < 10 * 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// --- new / load ---------------------------------------------------------------

fn create_new(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let g = parse_geometry(arg, "--new")?;
    if !g.has_w || !g.has_h || g.w.percent || g.h.percent || g.has_offset {
        return Err(Error::usage("--new: expected absolute WxH, e.g. 256x256"));
    }
    let (w, h) = (round_to_int(g.w.value), round_to_int(g.h.value));
    check_dimensions(w, h, "--new")?;
    let c = o.color("color", Color::new(0.0, 0.0, 0.0, 1.0))?;
    let (w, h) = (w as i32, h as i32);
    Ok(Op::new(move |ctx| {
        ctx.set_image(Image::filled(w, h, c));
        Ok(())
    }))
}

fn create_load(arg: &str, _o: &mut OptionReader) -> Result<Op> {
    let arg = arg.to_string();
    Ok(Op::new(move |ctx| {
        let path = ctx.expand_path(&arg)?;
        let (img, li) = load_image(&path)?;
        ctx.set_image(img);
        let msg = format!("loaded {path} ({}x{}, {})", ctx.image.width, ctx.image.height, li.format);
        ctx.verbose(msg);
        Ok(())
    }))
}

// --- save ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Tga,
    Dds,
    Png,
    Jpg,
    Bmp,
}

fn format_from_name(n: &str) -> Option<Format> {
    match n.to_ascii_lowercase().as_str() {
        "tga" => Some(Format::Tga),
        "dds" => Some(Format::Dds),
        "png" => Some(Format::Png),
        "jpg" | "jpeg" => Some(Format::Jpg),
        "bmp" => Some(Format::Bmp),
        _ => None,
    }
}

/// Parses "full", "none" or a total level count (1 = base level only).
fn parse_mip_levels(o: &mut OptionReader) -> Result<i32> {
    let s = o.str("mips", "full")?.to_ascii_lowercase();
    match s.as_str() {
        "full" | "all" => Ok(0),
        "none" | "0" => Ok(1),
        _ => match parse_long(&s) {
            Some(n) if (1..=17).contains(&n) => Ok(n as i32),
            _ => Err(Error::usage(format!("mips: expected full, none or a level count 1..17, got '{s}'"))),
        },
    }
}

/// Parses weights=perceptual|R,G,B into normalized channel weights.
fn parse_weights(w: &str) -> Result<Option<[f32; 3]>> {
    if w == "perceptual" {
        return Ok(None);
    }
    let bad = || Error::usage("weights: expected 'perceptual' or R,G,B (non-negative numbers)");
    let p = split(w, ',');
    if p.len() != 3 {
        return Err(bad());
    }
    let mut v = [0.0f64; 3];
    for (d, s) in v.iter_mut().zip(&p) {
        *d = parse_double(s).filter(|x| *x >= 0.0).ok_or_else(bad)?;
    }
    let sum = v[0] + v[1] + v[2];
    if sum <= 0.0 {
        return Err(bad());
    }
    // Normalized so the error metric scale stays comparable.
    Ok(Some(v.map(|x| (x / sum) as f32)))
}

/// Per-format settings, parsed up front.
#[derive(Clone, Copy)]
enum SaveSettings {
    Tga(TgaOptions),
    Dds(DdsOptions),
    Png { channels: i32, level: i32 },
    Jpg { quality: i32 },
    Bmp { channels: i32 },
}

fn create_save(arg: &str, o: &mut OptionReader) -> Result<Op> {
    if arg.is_empty() {
        return Err(Error::usage("--save: empty output path"));
    }
    let fmt_name = o.str("format", "auto")?;
    let fmt = if fmt_name.eq_ignore_ascii_case("auto") {
        format_from_name(&fs::extension(arg)).ok_or_else(|| {
            Error::usage(format!(
                "--save: cannot derive the format from '{arg}'; use a .tga/.dds/.png/.jpg/.bmp extension or format=..."
            ))
        })?
    } else {
        format_from_name(&fmt_name).ok_or_else(|| {
            Error::usage(format!("--save: unknown format '{fmt_name}' (expected tga, dds, png, jpg or bmp)"))
        })?
    };
    let mkdir = o.flag("mkdir", true)?;

    // Only the options that apply to the chosen format are read; finish()
    // then reports e.g. codec= on a TGA output as an error.
    let settings = match fmt {
        Format::Tga => {
            let bpp = o.choice("bpp", &["auto", "8", "24", "32"], 0)?;
            SaveSettings::Tga(TgaOptions {
                bpp: [0, 8, 24, 32][bpp],
                rle: o.flag("rle", false)?,
                top_origin: o.choice("origin", &["bottom", "top"], 0)? == 1,
            })
        }
        Format::Dds => {
            let mut dds = DdsOptions::default();
            let c = o.str("codec", "auto")?;
            dds.codec = parse_dds_codec(&c).ok_or_else(|| {
                Error::usage(format!("--save: unknown DDS codec '{c}' (expected {})", dds_codec_names().join(", ")))
            })?;
            dds.mips.levels = parse_mip_levels(o)?;
            dds.mips.min_size = o.integer("mip-min", 1, 1, MAX_DIMENSION)?;
            dds.mips.resample = o.resample_options(Filter::Box, EdgeMode::Repeat, "mip-")?;
            dds.mips.coverage_ref = o.number("mip-coverage", 0.0, 0.0, 1.0)? as f32;
            // BC7 gains under 1 dB PSNR from 0.1 to 1.0 on photographic
            // content at roughly 8x the encoding time; BC1-5 are fast at any
            // setting.
            let def_quality = if dds.codec == super::io_dds::DdsCodec::Bc7 { 0.1 } else { 1.0 };
            dds.block.quality = o.number("quality", def_quality, 0.0, 1.0)? as f32;
            dds.block.refine_steps = o.integer("refine", 0, 0, 16)?;
            if o.has("weights") {
                let w = o.str("weights", "")?.to_ascii_lowercase();
                dds.block.use_weights = true;
                if let Some(weights) = parse_weights(&w)? {
                    dds.block.weights = weights;
                }
            }
            dds.block.alpha_threshold = o.integer("alpha-threshold", 128, 1, 255)?;
            dds.dx10 = [-1, 0, 1][o.choice("dx10", &["auto", "0", "1"], 0)?];
            dds.srgb = o.flag("srgb", false)?;
            SaveSettings::Dds(dds)
        }
        Format::Png => {
            let level = o.integer("compression", 9, 0, 9)?;
            let channels = o.choice("channels", &["auto", "1", "2", "3", "4"], 0)? as i32;
            SaveSettings::Png { channels, level }
        }
        Format::Jpg => SaveSettings::Jpg { quality: o.integer("quality", 95, 1, 100)? },
        Format::Bmp => {
            let c = o.choice("channels", &["auto", "3", "4"], 0)? as i32;
            SaveSettings::Bmp { channels: if c == 0 { 0 } else { c + 2 } }
        }
    };

    let tmpl = arg.to_string();
    Ok(Op::with_output(
        move |ctx: &mut Context| {
            ctx.require_image("--save")?;
            let path = ctx.expand_path(&tmpl)?;
            if mkdir {
                let dir = fs::dirname(&path);
                if !dir.is_empty() {
                    fs::create_directories(&dir)?;
                }
            }
            let img = &ctx.image;
            let mut warning = None;
            let (bytes, desc) = match settings {
                SaveSettings::Tga(tga) => {
                    let (bytes, bpp) = encode_tga(img, &tga)?;
                    if tga.rle && bpp == 8 {
                        warning = Some(format!(
                            "warning: {path}: RLE grayscale TGAs (type 11) are not supported by the idTech 3 loader"
                        ));
                    }
                    (bytes, format!("tga {bpp}-bit{}", if tga.rle { " rle" } else { "" }))
                }
                SaveSettings::Dds(dds) => {
                    let r = encode_dds(img, &dds)?;
                    let desc = format!(
                        "dds {}, {} mip{}{}",
                        dds_codec_name(r.codec),
                        r.mip_levels,
                        if r.mip_levels == 1 { "" } else { "s" },
                        if r.dx10 { ", dx10" } else { "" }
                    );
                    (r.bytes, desc)
                }
                SaveSettings::Png { channels, level } => (encode_png(img, channels, level)?, "png".to_string()),
                SaveSettings::Jpg { quality } => (encode_jpg(img, quality)?, format!("jpg q{quality}")),
                SaveSettings::Bmp { channels } => (encode_bmp(img, channels)?, "bmp".to_string()),
            };
            let (w, h) = (img.width, img.height);
            if let Some(msg) = warning {
                ctx.info(msg);
            }
            fs::write_file(&path, &bytes)?;
            ctx.info(format!("wrote {path} ({w}x{h}, {desc}, {})", size_string(bytes.len())));
            Ok(())
        },
        arg,
    ))
}

// --- info ---------------------------------------------------------------------

fn create_info(_arg: &str, _o: &mut OptionReader) -> Result<Op> {
    Ok(Op::new(|ctx| {
        let img = ctx.require_image("--info")?;
        let a = analyze_alpha(img);
        let pot = is_power_of_two(img.width as i64) && is_power_of_two(img.height as i64);
        let gray = is_grayscale(img);
        let (w, h) = (img.width, img.height);
        let name = if ctx.input_path.is_empty() { "<image>".to_string() } else { ctx.input_path.clone() };
        let alpha = if a.opaque {
            "opaque"
        } else if a.binary {
            "binary (1-bit)"
        } else {
            "gradient"
        };
        ctx.print(format!(
            "{name}: {w}x{h}{}, alpha: {alpha} (min {}, max {}){}",
            if pot { " (power of two)" } else { " (not power of two)" },
            a.min,
            a.max,
            if gray { ", grayscale" } else { "" }
        ));
        Ok(())
    }))
}

// --- channel-from -------------------------------------------------------------

fn create_channel_from(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let from = o.choice("from", &["r", "g", "b", "a", "luma"], 4)?;
    let to = o.channels("to", 8)?;
    let resize = o.flag("resize", true)?;
    let rs = o.resample_options(Filter::Mitchell, EdgeMode::Clamp, "")?;
    let arg = arg.to_string();
    Ok(Op::new(move |ctx| {
        ctx.require_image("--channel-from")?;
        let path = ctx.expand_path(&arg)?;
        let (mut src, _) = load_image(&path)?;
        let img = &mut ctx.image;
        if src.width != img.width || src.height != img.height {
            if !resize {
                return Err(Error::runtime(format!(
                    "--channel-from: {path} is {}x{}, expected {}x{} (or use resize=1)",
                    src.width, src.height, img.width, img.height
                )));
            }
            src = resample(&src, img.width, img.height, &rs)?;
        }
        for (d, s) in img.data.chunks_exact_mut(4).zip(src.data.chunks_exact(4)) {
            let v = clamp01(if from == 4 { luma(s) } else { s[from] });
            for (c, dc) in d.iter_mut().enumerate() {
                if to & (1 << c) != 0 {
                    *dc = v;
                }
            }
        }
        Ok(())
    }))
}

// --- composite ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blend {
    Over,
    Add,
    Multiply,
    Screen,
    Replace,
}

fn create_composite(arg: &str, o: &mut OptionReader) -> Result<Op> {
    let gravity = o.gravity("gravity", Gravity::NorthWest)?;
    let ox = o.integer("x", 0, -MAX_DIMENSION, MAX_DIMENSION)? as i64;
    let oy = o.integer("y", 0, -MAX_DIMENSION, MAX_DIMENSION)? as i64;
    let blend = [Blend::Over, Blend::Add, Blend::Multiply, Blend::Screen, Blend::Replace]
        [o.choice("blend", &["over", "add", "multiply", "screen", "replace"], 0)?];
    let opacity = o.number("opacity", 1.0, 0.0, 1.0)? as f32;
    let tile = o.flag("tile", false)?;
    let arg = arg.to_string();
    Ok(Op::new(move |ctx| {
        ctx.require_image("--composite")?;
        let path = ctx.expand_path(&arg)?;
        let (src, _) = load_image(&path)?;
        let dst = &mut ctx.image;
        let (sw, sh) = (src.width as i64, src.height as i64);
        let (mut bx, mut by) = (0i64, 0i64);
        if !tile {
            (bx, by) = gravity_origin(gravity, dst.width as i64, dst.height as i64, sw, sh);
        }
        bx += ox;
        by += oy;
        for y in 0..dst.height {
            let mut sy = y as i64 - by;
            if tile {
                sy = sy.rem_euclid(sh);
            } else if sy < 0 || sy >= sh {
                continue;
            }
            for x in 0..dst.width {
                let mut sx = x as i64 - bx;
                if tile {
                    sx = sx.rem_euclid(sw);
                } else if sx < 0 || sx >= sw {
                    continue;
                }
                let s = src.pixel(sx as i32, sy as i32);
                let d = dst.pixel_mut(x, y);
                let sa = s[3] * opacity;
                let mut res = [0.0f32; 4];
                match blend {
                    Blend::Over => {
                        // Porter-Duff source-over on straight alpha.
                        let oa = sa + d[3] * (1.0 - sa);
                        for c in 0..3 {
                            res[c] = if oa > 0.0 { (s[c] * sa + d[c] * d[3] * (1.0 - sa)) / oa } else { 0.0 };
                        }
                        res[3] = oa;
                    }
                    Blend::Replace => {
                        for c in 0..4 {
                            res[c] = d[c] + (s[c] - d[c]) * opacity;
                        }
                    }
                    _ => {
                        for c in 0..3 {
                            let b = match blend {
                                Blend::Add => d[c] + s[c],
                                Blend::Multiply => d[c] * s[c],
                                _ => 1.0 - (1.0 - d[c]) * (1.0 - s[c]),
                            };
                            res[c] = d[c] + (clamp01(b) - d[c]) * sa;
                        }
                        res[3] = d[3];
                    }
                }
                for c in 0..4 {
                    d[c] = clamp01(res[c]);
                }
            }
        }
        Ok(())
    }))
}

fn with(mut base: Vec<OptionHelp>, extra: Vec<OptionHelp>) -> Vec<OptionHelp> {
    base.extend(extra);
    base
}

pub fn ops() -> Vec<OpInfo> {
    let cat = "Input & output";
    let info = |name, aliases, arg, summary: &str, details: &str, options, create| OpInfo {
        name,
        aliases,
        arg,
        category: cat,
        summary: summary.to_string(),
        details: details.to_string(),
        options,
        create,
    };
    vec![
        info("new", &[], "WxH", "start from a blank image", "", vec![opt("color", "black", "fill color")], create_new),
        info(
            "load",
            &[],
            "FILE",
            "replace the current image with FILE (path placeholders are expanded)",
            "",
            vec![],
            create_load,
        ),
        info(
            "save",
            &["o", "output"],
            "PATH",
            "write the current image; the pipeline continues afterwards",
            "The format follows the extension (.tga, .dds, .png, .jpg, .bmp) unless format= is given. PATH may \
             contain placeholders: {name} input file stem, {ext} input extension, {dir} input directory, {rel} \
             path below a directory input (without extension), {index}, {w}, {h}; {{ and }} for literal braces. \
             Only the options of the chosen format are accepted.",
            with(
                vec![
                    opt("format", "auto", "tga, dds, png, jpg, bmp"),
                    opt("mkdir", "1", "create missing parent directories"),
                    opt("bpp", "auto", "TGA: auto (32 with alpha, else 24), 8 (grayscale), 24, 32"),
                    opt("rle", "0", "TGA: run-length compression (supported by idTech 3 for 24/32 bit)"),
                    opt("origin", "bottom", "TGA: row order; ioquake3 ignores the flag, keep bottom for idTech 3"),
                    opt(
                        "codec",
                        "auto",
                        &format!("DDS: {}; auto = dxt1 when opaque, else dxt5", dds_codec_names().join(", ")),
                    ),
                    opt("mips", "full", "DDS: full, none or total level count"),
                    opt("mip-min", "1", "DDS: smallest mip dimension to generate"),
                ],
                with(resample_option_help("box", "repeat", "mip-"), vec![
                    opt(
                        "mip-coverage",
                        "0",
                        "DDS: keep alpha-test coverage in mips at this alpha reference (e.g. 0.5); 0 = off",
                    ),
                    opt(
                        "quality",
                        "",
                        "DDS: block compression quality 0..1, lower is faster (default 1, bc7: 0.1); JPG: 1..100 \
                         (default 95)",
                    ),
                    opt("refine", "0", "DDS: extra BC1 endpoint refinement steps"),
                    opt("weights", "", "DDS: color error weights 'perceptual' or R,G,B (BC1-3)"),
                    opt("alpha-threshold", "128", "DDS bc1a: alpha below this becomes transparent"),
                    opt("dx10", "auto", "DDS: write the DX10 header: auto (only when needed), 0, 1"),
                    opt("srgb", "0", "DDS: tag the format as sRGB (requires the DX10 header)"),
                    opt("compression", "9", "PNG: zlib level 0..9"),
                    opt("channels", "auto", "PNG: auto, 1 (gray), 2 (gray+alpha), 3, 4; BMP: auto, 3, 4"),
                ]),
            ),
            create_save,
        ),
        info("info", &[], "", "print size and alpha information to stdout", "", vec![], create_info),
        info(
            "channel-from",
            &[],
            "FILE",
            "copy one channel of FILE into channels of the current image",
            "Typical use: --channel-from mask.png from=luma to=a to attach a separate alpha mask.",
            with(
                vec![
                    opt("from", "luma", "source channel: r, g, b, a or luma"),
                    opt("to", "a", "destination channels, any of r, g, b, a"),
                    opt("resize", "1", "resample FILE when its size differs"),
                ],
                resample_option_help("mitchell", "clamp", ""),
            ),
            create_channel_from,
        ),
        info(
            "composite",
            &["overlay"],
            "FILE",
            "draw FILE on top of the current image",
            "",
            vec![
                opt("gravity", "nw", GRAVITY_HELP),
                opt("x", "0", "horizontal offset in pixels"),
                opt("y", "0", "vertical offset in pixels"),
                opt("blend", "over", "over, add, multiply, screen, replace"),
                opt("opacity", "1", "0..1"),
                opt("tile", "0", "repeat FILE over the whole image"),
            ],
            create_composite,
        ),
    ]
}
