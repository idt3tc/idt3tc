//! Separable resampling filters and Gaussian blur.
//!
//! All transcendental math goes through the `libm` crate and filter weights
//! are computed in f64 exactly as before, so results are identical on every
//! platform. Parallelism uses the current rayon pool.

use rayon::prelude::*;

use super::image::{Color, Image, check_dimensions, clamp_image, premultiply, srgb_to_linear, to_linear, to_srgb, unpremultiply};
use super::sampling::{EdgeMode, resolve_edge};
use crate::common::{Error, Result};

/// Reconstruction filters for resampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Box,
    /// a.k.a. bilinear / tent
    Triangle,
    Hermite,
    /// Cubic B-spline (B=1, C=0), very smooth.
    BSpline,
    /// Mitchell-Netravali (B=1/3, C=1/3).
    Mitchell,
    /// B=0, C=1/2, sharper bicubic.
    CatmullRom,
    Gaussian,
    Lanczos2,
    Lanczos3,
    /// Windowed sinc with a Kaiser window (radius 3).
    Kaiser,
}

/// Name and support radius (in source pixels at scale 1) of every filter,
/// in help output order.
const FILTERS: &[(Filter, &str, f64)] = &[
    (Filter::Nearest, "nearest", 0.5),
    (Filter::Box, "box", 0.5),
    (Filter::Triangle, "triangle", 1.0),
    (Filter::Hermite, "hermite", 1.0),
    (Filter::BSpline, "bspline", 2.0),
    (Filter::Mitchell, "mitchell", 2.0),
    (Filter::CatmullRom, "catmullrom", 2.0),
    (Filter::Gaussian, "gaussian", 1.5),
    (Filter::Lanczos2, "lanczos2", 2.0),
    (Filter::Lanczos3, "lanczos3", 3.0),
    (Filter::Kaiser, "kaiser", 3.0),
];

fn filter_support(f: Filter) -> f64 {
    FILTERS.iter().find(|d| d.0 == f).map_or(0.5, |d| d.2)
}

/// Parses a filter name, accepting common aliases from other tools.
pub fn parse_filter(name: &str) -> Option<Filter> {
    let n = name.to_ascii_lowercase();
    let n = match n.as_str() {
        "point" => "nearest",
        "bilinear" | "linear" | "tent" => "triangle",
        "bicubic" | "cubic" => "catmullrom",
        "lanczos" => "lanczos3",
        other => other,
    };
    FILTERS.iter().find(|d| d.1 == n).map(|d| d.0)
}

pub fn filter_name(f: Filter) -> &'static str {
    FILTERS.iter().find(|d| d.0 == f).map_or("nearest", |d| d.1)
}

pub fn filter_names() -> Vec<&'static str> {
    FILTERS.iter().map(|d| d.1).collect()
}

#[derive(Debug, Clone, Copy)]
pub struct ResampleOptions {
    pub filter: Filter,
    pub edge: EdgeMode,
    /// Used with EdgeMode::Fill.
    pub fill: Color,
    /// Filter in linear light (sRGB decode/encode around it).
    pub linear: bool,
    /// Weight colors by alpha to avoid dark/colored fringes.
    pub premultiply: bool,
    /// >1 widens the filter (softer), <1 narrows it (sharper).
    pub blur: f32,
}

impl Default for ResampleOptions {
    fn default() -> Self {
        ResampleOptions {
            filter: Filter::Mitchell,
            edge: EdgeMode::Clamp,
            fill: Color::TRANSPARENT,
            linear: false,
            premultiply: true,
            blur: 1.0,
        }
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-8 {
        return 1.0;
    }
    let x = x * std::f64::consts::PI;
    libm::sin(x) / x
}

/// Generic cubic from Mitchell & Netravali.
fn cubic_bc(x: f64, b: f64, c: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * x * x * x + (-18.0 + 12.0 * b + 6.0 * c) * x * x + (6.0 - 2.0 * b)) / 6.0
    } else if x < 2.0 {
        ((-b - 6.0 * c) * x * x * x + (6.0 * b + 30.0 * c) * x * x + (-12.0 * b - 48.0 * c) * x + (8.0 * b + 24.0 * c))
            / 6.0
    } else {
        0.0
    }
}

/// Zeroth order modified Bessel function of the first kind (series).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-12 {
            break;
        }
    }
    sum
}

fn eval_filter(f: Filter, x: f64) -> f64 {
    let ax = x.abs();
    match f {
        // Half-open so that integer downscales average exactly k samples.
        Filter::Nearest | Filter::Box => {
            if (-0.5..0.5).contains(&x) {
                1.0
            } else {
                0.0
            }
        }
        Filter::Triangle => {
            if ax < 1.0 {
                1.0 - ax
            } else {
                0.0
            }
        }
        Filter::Hermite => {
            if ax < 1.0 {
                (2.0 * ax - 3.0) * ax * ax + 1.0
            } else {
                0.0
            }
        }
        Filter::BSpline => cubic_bc(x, 1.0, 0.0),
        Filter::Mitchell => cubic_bc(x, 1.0 / 3.0, 1.0 / 3.0),
        Filter::CatmullRom => cubic_bc(x, 0.0, 0.5),
        Filter::Gaussian => {
            if ax < 1.5 {
                libm::exp(-2.0 * x * x)
            } else {
                0.0
            }
        }
        Filter::Lanczos2 => {
            if ax < 2.0 {
                sinc(x) * sinc(x / 2.0)
            } else {
                0.0
            }
        }
        Filter::Lanczos3 => {
            if ax < 3.0 {
                sinc(x) * sinc(x / 3.0)
            } else {
                0.0
            }
        }
        Filter::Kaiser => {
            if ax >= 3.0 {
                return 0.0;
            }
            let alpha = 4.0;
            let r = x / 3.0;
            sinc(x) * bessel_i0(alpha * libm::sqrt((1.0 - r * r).max(0.0))) / bessel_i0(alpha)
        }
    }
}

/// Contributions of source samples to each destination sample, stored
/// flat: destination i uses taps start[i]..start[i+1]. A source index of -1
/// stands for the fill color (EdgeMode::Fill outside the image).
pub(crate) struct Contribs {
    start: Vec<usize>,
    index: Vec<i32>,
    weight: Vec<f32>,
}

impl Contribs {
    fn new() -> Self {
        Contribs { start: vec![0], index: Vec::new(), weight: Vec::new() }
    }

    fn len(&self) -> usize {
        self.start.len() - 1
    }

    fn taps(&self, i: usize) -> impl Iterator<Item = (i32, f32)> + '_ {
        let r = self.start[i]..self.start[i + 1];
        self.index[r.clone()].iter().copied().zip(self.weight[r].iter().copied())
    }

    fn push(&mut self, index: Option<i32>, weight: f32) {
        self.index.push(index.unwrap_or(-1));
        self.weight.push(weight);
    }

    fn finish_sample(&mut self) {
        self.start.push(self.index.len());
    }
}

fn compute_contribs(src_size: i32, dst_size: i32, o: &ResampleOptions) -> Contribs {
    let mut out = Contribs::new();
    let scale = dst_size as f64 / src_size as f64;
    let clamp_index = |v: f64| (libm::floor(v) as i64).clamp(0, src_size as i64 - 1) as i32;
    if o.filter == Filter::Nearest {
        for i in 0..dst_size {
            out.push(Some(clamp_index((i as f64 + 0.5) / scale)), 1.0);
            out.finish_sample();
        }
        return out;
    }
    // When minifying, stretch the filter so it covers all source pixels
    // that fall into a destination pixel (avoids aliasing).
    let filter_scale = (1.0 / scale).max(1.0) * o.blur.max(0.01) as f64;
    let support = filter_support(o.filter) * filter_scale;
    for i in 0..dst_size {
        let center = (i as f64 + 0.5) / scale - 0.5;
        let first = libm::ceil(center - support) as i64;
        let last = libm::floor(center + support) as i64;
        let begin = out.index.len();
        let mut total = 0.0f64;
        for s in first..=last {
            let w = eval_filter(o.filter, (s as f64 - center) / filter_scale);
            if w == 0.0 {
                continue;
            }
            out.push(resolve_edge(s, src_size, o.edge), w as f32);
            total += w;
        }
        if out.index.len() == begin || total.abs() < 1e-12 {
            // Degenerate kernel (can happen with tiny blur values); fall
            // back to the nearest sample.
            out.index.truncate(begin);
            out.weight.truncate(begin);
            out.push(Some(clamp_index(center + 0.5)), 1.0);
        } else {
            for w in &mut out.weight[begin..] {
                *w = (*w as f64 / total) as f32;
            }
        }
        out.finish_sample();
    }
    out
}

/// Applies horizontal then vertical contributions.
fn convolve_separable(src: &Image, cx: &Contribs, cy: &Contribs, fill: &Color) -> Image {
    let dw = cx.len() as i32;
    let dh = cy.len() as i32;
    let fillv = fill.to_array();

    // Horizontal pass: src.height rows of dw pixels.
    let mut tmp = Image::new(dw, src.height);
    let tmp_stride = tmp.stride();
    tmp.data.par_chunks_mut(tmp_stride).enumerate().for_each(|(y, drow)| {
        let srow = src.row(y as i32);
        for (x, d) in drow.chunks_exact_mut(4).enumerate() {
            let mut acc = [0.0f32; 4];
            for (idx, w) in cx.taps(x) {
                let p = if idx < 0 { &fillv[..] } else { &srow[idx as usize * 4..idx as usize * 4 + 4] };
                acc[0] += p[0] * w;
                acc[1] += p[1] * w;
                acc[2] += p[2] * w;
                acc[3] += p[3] * w;
            }
            d.copy_from_slice(&acc);
        }
    });

    // Vertical pass.
    let mut dst = Image::new(dw, dh);
    let stride = dst.stride();
    dst.data.par_chunks_mut(stride).enumerate().for_each(|(y, acc)| {
        acc.fill(0.0);
        for (idx, w) in cy.taps(y) {
            if idx < 0 {
                for px in acc.chunks_exact_mut(4) {
                    for ch in 0..4 {
                        px[ch] += fillv[ch] * w;
                    }
                }
                continue;
            }
            let srow = tmp.row(idx);
            for (a, s) in acc.iter_mut().zip(srow) {
                *a += *s * w;
            }
        }
    });
    dst
}

/// Shared pre/post processing around a separable filter.
fn filter_image(src: &Image, cx: &Contribs, cy: &Contribs, mut fill: Color, linear: bool, premul: bool) -> Image {
    let mut work = src.clone();
    if linear {
        to_linear(&mut work);
        fill.r = srgb_to_linear(fill.r);
        fill.g = srgb_to_linear(fill.g);
        fill.b = srgb_to_linear(fill.b);
    }
    if premul {
        premultiply(&mut work);
        fill.r *= fill.a;
        fill.g *= fill.a;
        fill.b *= fill.a;
    }
    let mut dst = convolve_separable(&work, cx, cy, &fill);
    // Negative lobes (Lanczos, Catmull-Rom, ...) may overshoot.
    clamp_image(&mut dst);
    if premul {
        unpremultiply(&mut dst);
    }
    if linear {
        to_srgb(&mut dst);
    }
    dst
}

/// Resamples `src` to w x h with separable filtering.
pub fn resample(src: &Image, w: i32, h: i32, opts: &ResampleOptions) -> Result<Image> {
    check_dimensions(w as i64, h as i64, "resample")?;
    if src.is_empty() {
        return Err(Error::runtime("resample: empty source image"));
    }
    // Interpolating filters at scale 1 are the identity.
    let interpolating = matches!(
        opts.filter,
        Filter::Nearest
            | Filter::Box
            | Filter::Triangle
            | Filter::Hermite
            | Filter::Lanczos2
            | Filter::Lanczos3
            | Filter::CatmullRom
            | Filter::Kaiser
    );
    if w == src.width && h == src.height && (opts.filter == Filter::Nearest || opts.blur == 1.0) && interpolating {
        return Ok(src.clone());
    }
    let cx = compute_contribs(src.width, w, opts);
    let cy = compute_contribs(src.height, h, opts);
    Ok(filter_image(
        src,
        &cx,
        &cy,
        opts.fill,
        opts.linear,
        opts.premultiply && opts.filter != Filter::Nearest,
    ))
}

/// Separable Gaussian convolution with standard deviation `sigma` (pixels).
/// Alpha-weighted when `premultiply_alpha` is set.
pub fn gaussian_blur(src: &Image, sigma: f32, edge: EdgeMode, fill: &Color, premultiply_alpha: bool) -> Image {
    if !(sigma > 0.0) {
        return src.clone();
    }
    let radius = (libm::ceilf(sigma * 3.0) as i32).max(1);
    let sigma = sigma as f64;
    let mut kernel = Vec::with_capacity(2 * radius as usize + 1);
    let mut total = 0.0f64;
    for i in -radius..=radius {
        let w = libm::exp(-(i as f64 * i as f64) / (2.0 * sigma * sigma));
        kernel.push(w as f32);
        total += w;
    }
    for k in &mut kernel {
        *k = (*k as f64 / total) as f32;
    }
    let build = |n: i32| {
        let mut cs = Contribs::new();
        for i in 0..n {
            for k in -radius..=radius {
                cs.push(resolve_edge(i as i64 + k as i64, n, edge), kernel[(k + radius) as usize]);
            }
            cs.finish_sample();
        }
        cs
    };
    filter_image(src, &build(src.width), &build(src.height), *fill, false, premultiply_alpha)
}
