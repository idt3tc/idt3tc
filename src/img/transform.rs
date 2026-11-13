//! Geometric transforms: regions, lossless rotations and flips, arbitrary
//! rotation and translation.

use rayon::prelude::*;

use super::image::{Color, Image, check_dimensions, clamp_image, premultiply, unpremultiply};
use super::sampling::{EdgeMode, Interp, fetch_pixel, sample_pixel};
use crate::common::Result;

#[derive(Debug, Clone, Copy)]
pub struct AffineOptions {
    pub interp: Interp,
    pub edge: EdgeMode,
    pub fill: Color,
    pub premultiply: bool,
}

impl Default for AffineOptions {
    fn default() -> Self {
        AffineOptions { interp: Interp::Bicubic, edge: EdgeMode::Fill, fill: Color::TRANSPARENT, premultiply: true }
    }
}

/// Inverse-maps every destination pixel center through `map` and samples
/// the (optionally premultiplied) source.
fn warp(src: &Image, w: i32, h: i32, opts: &AffineOptions, map: impl Fn(f64, f64) -> (f64, f64) + Sync) -> Image {
    let mut work = src.clone();
    let mut fill = opts.fill;
    let premul = opts.premultiply && opts.interp != Interp::Nearest;
    if premul {
        premultiply(&mut work);
        fill.r *= fill.a;
        fill.g *= fill.a;
        fill.b *= fill.a;
    }
    let mut dst = Image::new(w, h);
    let stride = dst.stride();
    dst.data.par_chunks_mut(stride).enumerate().for_each(|(y, row)| {
        for (x, d) in row.chunks_exact_mut(4).enumerate() {
            let (sx, sy) = map(x as f64 + 0.5, y as f64 + 0.5);
            d.copy_from_slice(&sample_pixel(&work, sx as f32, sy as f32, opts.interp, opts.edge, &fill));
        }
    });
    clamp_image(&mut dst);
    if premul {
        unpremultiply(&mut dst);
    }
    dst
}

/// Copies the w x h region at (x, y); parts outside the source are resolved
/// through `edge` (fill, clamp, repeat or mirror).
pub fn extract_region(src: &Image, x: i64, y: i64, w: i32, h: i32, edge: EdgeMode, fill: &Color) -> Result<Image> {
    check_dimensions(w as i64, h as i64, "region")?;
    let mut dst = Image::new(w, h);
    for j in 0..h {
        for i in 0..w {
            let p = fetch_pixel(src, x + i as i64, y + j as i64, edge, fill);
            dst.pixel_mut(i, j).copy_from_slice(&p);
        }
    }
    Ok(dst)
}

/// Remaps every pixel: the source pixel (x, y) lands at `to(x, y)` in a
/// destination of dw x dh.
fn remap(src: &Image, dw: i32, dh: i32, to: impl Fn(i32, i32) -> (i32, i32)) -> Image {
    let mut dst = Image::new(dw, dh);
    for y in 0..src.height {
        for x in 0..src.width {
            let (dx, dy) = to(x, y);
            dst.pixel_mut(dx, dy).copy_from_slice(src.pixel(x, y));
        }
    }
    dst
}

/// Lossless quarter turns; `quarters` is taken modulo 4, clockwise.
pub fn rotate_quarter(src: &Image, quarters: i32) -> Image {
    let (w, h) = (src.width, src.height);
    match quarters.rem_euclid(4) {
        0 => src.clone(),
        1 => remap(src, h, w, |x, y| (h - 1 - y, x)),
        2 => remap(src, w, h, |x, y| (w - 1 - x, h - 1 - y)),
        _ => remap(src, h, w, |x, y| (y, w - 1 - x)),
    }
}

pub fn flip_horizontal(src: &Image) -> Image {
    let w = src.width;
    remap(src, w, src.height, |x, y| (w - 1 - x, y))
}

pub fn flip_vertical(src: &Image) -> Image {
    let mut dst = Image::new(src.width, src.height);
    for y in 0..src.height {
        dst.row_mut(src.height - 1 - y).copy_from_slice(src.row(y));
    }
    dst
}

pub fn transpose(src: &Image) -> Image {
    remap(src, src.height, src.width, |x, y| (y, x))
}

/// Rotates by `degrees` clockwise around the image center. With `expand`
/// the canvas grows to contain the whole rotated image.
pub fn rotate_arbitrary(src: &Image, degrees: f64, expand: bool, opts: &AffineOptions) -> Result<Image> {
    let rad = degrees * std::f64::consts::PI / 180.0;
    let (c, s) = (libm::cos(rad), libm::sin(rad));
    let (mut w, mut h) = (src.width as i64, src.height as i64);
    if expand {
        // Bounding box of the rotated rectangle; the epsilon avoids growing
        // by a pixel due to floating point noise.
        let (sw, sh) = (src.width as f64, src.height as f64);
        let bw = (sw * c).abs() + (sh * s).abs();
        let bh = (sw * s).abs() + (sh * c).abs();
        w = (libm::ceil(bw - 1e-6) as i64).max(1);
        h = (libm::ceil(bh - 1e-6) as i64).max(1);
    }
    check_dimensions(w, h, "rotate")?;
    let (scx, scy) = (src.width as f64 / 2.0, src.height as f64 / 2.0);
    let (dcx, dcy) = (w as f64 / 2.0, h as f64 / 2.0);
    // With y pointing down, a clockwise rotation on screen is the standard
    // rotation matrix; its inverse maps destination back to source.
    Ok(warp(src, w as i32, h as i32, opts, |x, y| {
        let (rx, ry) = (x - dcx, y - dcy);
        (c * rx + s * ry + scx, -s * rx + c * ry + scy)
    }))
}

/// Shifts content by (dx, dy) pixels (fractional allowed); with
/// EdgeMode::Repeat this is the classic texture "offset" for checking seams.
pub fn translate(src: &Image, dx: f64, dy: f64, opts: &AffineOptions) -> Image {
    // Integral shifts are pure index remapping; avoid filtering entirely.
    if dx == libm::floor(dx) && dy == libm::floor(dy) {
        let (ix, iy) = (dx as i64, dy as i64);
        let mut dst = Image::new(src.width, src.height);
        for y in 0..src.height {
            for x in 0..src.width {
                let p = fetch_pixel(src, x as i64 - ix, y as i64 - iy, opts.edge, &opts.fill);
                dst.pixel_mut(x, y).copy_from_slice(&p);
            }
        }
        return dst;
    }
    warp(src, src.width, src.height, opts, |x, y| (x - dx, y - dy))
}
