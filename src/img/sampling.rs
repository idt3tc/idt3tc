//! Pixel access beyond the image border and sub-pixel interpolation.

use super::image::{Color, Image};

/// How coordinates outside the image are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    /// Repeat the outermost pixels.
    Clamp,
    /// Wrap around (tiling textures).
    Repeat,
    /// Reflect at the border.
    Mirror,
    /// Use a constant fill color.
    Fill,
}

/// Interpolation used for arbitrary (sub-pixel) sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Nearest,
    Bilinear,
    Bicubic,
}

/// Maps an integer coordinate into [0, n) according to `mode`. Returns None
/// for EdgeMode::Fill when `i` is outside the image. Coordinates are i64 so
/// that offsets added to saturated float conversions cannot overflow.
#[inline]
pub fn resolve_edge(i: i64, n: i32, mode: EdgeMode) -> Option<i32> {
    let n64 = n as i64;
    if i >= 0 && i < n64 {
        return Some(i as i32);
    }
    match mode {
        EdgeMode::Clamp => Some(if i < 0 { 0 } else { n - 1 }),
        EdgeMode::Repeat => Some(i.rem_euclid(n64) as i32),
        EdgeMode::Mirror => {
            // Period of 2n: 0..n-1 forward, then n-1..0 backwards.
            let period = 2 * n64;
            let m = i.rem_euclid(period);
            Some(if m < n64 { m } else { period - 1 - m } as i32)
        }
        EdgeMode::Fill => None,
    }
}

/// Reads a pixel honoring the edge mode.
#[inline]
pub fn fetch_pixel(img: &Image, x: i64, y: i64, edge: EdgeMode, fill: &Color) -> [f32; 4] {
    match (resolve_edge(x, img.width, edge), resolve_edge(y, img.height, edge)) {
        (Some(sx), Some(sy)) => {
            let p = img.pixel(sx, sy);
            [p[0], p[1], p[2], p[3]]
        }
        _ => fill.to_array(),
    }
}

/// Catmull-Rom spline weights for fractional offset t in [0, 1).
fn catmull_rom_weights(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// Float to integer the way a C cast of floor() behaves for sane values;
/// out-of-range and NaN inputs saturate instead of being undefined.
#[inline]
fn floor_i64(v: f32) -> i64 {
    libm::floorf(v) as i64
}

/// Samples `img` at continuous coordinates where pixel (i, j) covers
/// [i, i+1) x [j, j+1), i.e. its center is at (i + 0.5, j + 0.5).
pub fn sample_pixel(img: &Image, x: f32, y: f32, interp: Interp, edge: EdgeMode, fill: &Color) -> [f32; 4] {
    if interp == Interp::Nearest {
        return fetch_pixel(img, floor_i64(x), floor_i64(y), edge, fill);
    }
    // Shift to pixel-center coordinates.
    let fx = x - 0.5;
    let fy = y - 0.5;
    let ix = floor_i64(fx);
    let iy = floor_i64(fy);
    let tx = fx - ix as f32;
    let ty = fy - iy as f32;
    let mut acc = [0.0f32; 4];
    let mut add = |px: i64, py: i64, w: f32| {
        if w == 0.0 {
            return;
        }
        let p = fetch_pixel(img, px, py, edge, fill);
        for c in 0..4 {
            acc[c] += p[c] * w;
        }
    };
    if interp == Interp::Bilinear {
        let wx = [1.0 - tx, tx];
        let wy = [1.0 - ty, ty];
        for j in 0..2 {
            for i in 0..2 {
                add(ix + i as i64, iy + j as i64, wx[i] * wy[j]);
            }
        }
    } else {
        let wx = catmull_rom_weights(tx);
        let wy = catmull_rom_weights(ty);
        for j in 0..4 {
            for i in 0..4 {
                add(ix - 1 + i as i64, iy - 1 + j as i64, wx[i] * wy[j]);
            }
        }
    }
    acc
}
