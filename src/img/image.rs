//! The working image representation and basic per-pixel conversions.

use crate::common::{Error, Result};

/// An RGBA color with float channels, nominally in [0, 1].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    pub const TRANSPARENT: Color = Color::new(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);

    pub fn get(&self, i: usize) -> f32 {
        match i {
            0 => self.r,
            1 => self.g,
            2 => self.b,
            _ => self.a,
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Working image: 4 float channels (RGBA), rows stored top to bottom.
///
/// All operations work in this representation so that chains of operations
/// do not accumulate 8-bit rounding errors. Color values are kept in the
/// encoding they were loaded in (normally sRGB); operations that care about
/// light-linear math offer an explicit `linear` option.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Image {
    pub width: i32,
    pub height: i32,
    pub data: Vec<f32>,
}

/// Maximum supported dimension; guards against absurd allocations from
/// typos like "--resize 100000x100000".
pub const MAX_DIMENSION: i32 = 65536;

/// Maximum number of pixels of one image (16384x16384, 4 GiB of float
/// RGBA). Both dimensions may be large, but not at the same time, so a
/// hostile header or option cannot request tens of gigabytes.
pub const MAX_PIXELS: u64 = 1 << 28;

/// Fails unless w/h are in [1, MAX_DIMENSION] and the pixel count is at
/// most MAX_PIXELS.
pub fn check_dimensions(w: i64, h: i64, what: &str) -> Result<()> {
    if w < 1 || h < 1 || w > MAX_DIMENSION as i64 || h > MAX_DIMENSION as i64 {
        return Err(Error::runtime(format!(
            "{what}: invalid image size {w}x{h} (allowed: 1..{MAX_DIMENSION})"
        )));
    }
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(Error::runtime(format!(
            "{what}: image size {w}x{h} exceeds the limit of {MAX_PIXELS} pixels"
        )));
    }
    Ok(())
}

impl Image {
    /// A zeroed (transparent black) image. Callers validate the size with
    /// check_dimensions first.
    pub fn new(w: i32, h: i32) -> Self {
        let n = (w.max(0) as usize) * (h.max(0) as usize) * 4;
        Image { width: w, height: h, data: vec![0.0; n] }
    }

    pub fn filled(w: i32, h: i32, fill: Color) -> Self {
        let mut img = Image::new(w, h);
        if fill != Color::TRANSPARENT {
            for px in img.data.chunks_exact_mut(4) {
                px.copy_from_slice(&fill.to_array());
            }
        }
        img
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn pixel_count(&self) -> usize {
        self.data.len() / 4
    }

    fn offset(&self, x: i32, y: i32) -> usize {
        debug_assert!(x >= 0 && y >= 0 && x < self.width && y < self.height);
        (y as usize * self.width as usize + x as usize) * 4
    }

    pub fn pixel(&self, x: i32, y: i32) -> &[f32] {
        let o = self.offset(x, y);
        &self.data[o..o + 4]
    }

    pub fn pixel_mut(&mut self, x: i32, y: i32) -> &mut [f32] {
        let o = self.offset(x, y);
        &mut self.data[o..o + 4]
    }

    pub fn row(&self, y: i32) -> &[f32] {
        let w = self.width as usize * 4;
        &self.data[y as usize * w..(y as usize + 1) * w]
    }

    pub fn row_mut(&mut self, y: i32) -> &mut [f32] {
        let w = self.width as usize * 4;
        &mut self.data[y as usize * w..(y as usize + 1) * w]
    }

    /// Bytes per row of the float buffer, in elements; handy for parallel
    /// row chunking.
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

/// Float to 8 bit with rounding and clamping (NaN gives 0).
#[inline]
pub fn to_byte(v: f32) -> u8 {
    if !(v > 0.0) {
        0
    } else if v >= 1.0 {
        255
    } else {
        (v * 255.0 + 0.5) as u8
    }
}

#[inline]
pub fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        libm::powf((v + 0.055) / 1.055, 2.4)
    }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * libm::powf(v, 1.0 / 2.4) - 0.055
    }
}

/// sRGB to linear light, RGB channels only.
pub fn to_linear(img: &mut Image) {
    for px in img.data.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = srgb_to_linear(*c);
        }
    }
}

/// Linear light to sRGB, RGB channels only; negative values become 0.
pub fn to_srgb(img: &mut Image) {
    for px in img.data.chunks_exact_mut(4) {
        for c in &mut px[..3] {
            *c = linear_to_srgb(if *c < 0.0 { 0.0 } else { *c });
        }
    }
}

pub fn premultiply(img: &mut Image) {
    for px in img.data.chunks_exact_mut(4) {
        let a = px[3];
        px[0] *= a;
        px[1] *= a;
        px[2] *= a;
    }
}

/// Inverse of premultiply; fully transparent pixels get black RGB.
pub fn unpremultiply(img: &mut Image) {
    for px in img.data.chunks_exact_mut(4) {
        let a = px[3];
        if a > 1e-6 {
            px[0] /= a;
            px[1] /= a;
            px[2] /= a;
        } else {
            px[0] = 0.0;
            px[1] = 0.0;
            px[2] = 0.0;
        }
    }
}

pub fn clamp_image(img: &mut Image) {
    for v in &mut img.data {
        *v = clamp01(*v);
    }
}

/// Quantized RGBA8 buffer (row-major, top-down).
pub fn to_rgba8(img: &Image) -> Vec<u8> {
    img.data.iter().map(|&v| to_byte(v)).collect()
}

/// Image from an RGBA8 buffer of exactly w*h*4 bytes.
pub fn from_rgba8(rgba: &[u8], w: i32, h: i32) -> Image {
    let mut img = Image::new(w, h);
    for (d, &s) in img.data.iter_mut().zip(rgba) {
        *d = s as f32 / 255.0;
    }
    img
}

/// Alpha characteristics after 8-bit quantization, used to pick output
/// formats automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlphaInfo {
    /// Every alpha is 255.
    pub opaque: bool,
    /// Every alpha is 0 or 255.
    pub binary: bool,
    pub min: u8,
    pub max: u8,
}

pub fn analyze_alpha(img: &Image) -> AlphaInfo {
    let mut info = AlphaInfo { opaque: true, binary: true, min: 255, max: 0 };
    for px in img.data.chunks_exact(4) {
        let a = to_byte(px[3]);
        if a != 255 {
            info.opaque = false;
        }
        if a != 0 && a != 255 {
            info.binary = false;
        }
        info.min = info.min.min(a);
        info.max = info.max.max(a);
    }
    if img.data.is_empty() {
        info.min = 255;
        info.max = 255;
    }
    info
}

/// True if R == G == B for every pixel after 8-bit quantization.
pub fn is_grayscale(img: &Image) -> bool {
    img.data.chunks_exact(4).all(|px| {
        let r = to_byte(px[0]);
        to_byte(px[1]) == r && to_byte(px[2]) == r
    })
}

pub fn is_power_of_two(v: i64) -> bool {
    v > 0 && (v & (v - 1)) == 0
}

/// Rec. 709 luma of an RGB triple.
#[inline]
pub fn luma(p: &[f32]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}
