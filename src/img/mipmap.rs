//! Mipmap chain generation with optional alpha-test coverage preservation.

use super::image::{Image, clamp01};
use super::resample::{Filter, ResampleOptions, resample};
use super::sampling::EdgeMode;
use crate::common::Result;

#[derive(Debug, Clone, Copy)]
pub struct MipOptions {
    /// Total levels incl. base; 0 = full chain down to 1x1.
    pub levels: i32,
    /// Stop before the larger dimension drops below this.
    pub min_size: i32,
    pub resample: ResampleOptions,
    /// >0: preserve alpha-test coverage at this alpha ref.
    pub coverage_ref: f32,
}

impl Default for MipOptions {
    fn default() -> Self {
        MipOptions {
            levels: 0,
            min_size: 1,
            resample: ResampleOptions {
                filter: Filter::Box,
                // Textures in idTech 3 almost always tile.
                edge: EdgeMode::Repeat,
                ..ResampleOptions::default()
            },
            coverage_ref: 0.0,
        }
    }
}

/// Number of levels of a full chain for w x h (down to 1x1).
pub fn full_mip_count(w: i32, h: i32) -> i32 {
    let mut n = 1;
    let mut m = w.max(h);
    while m > 1 {
        m >>= 1;
        n += 1;
    }
    n
}

/// Returns base + generated levels. Every level is filtered directly from
/// the base image (no cumulative blur from repeated halving).
pub fn build_mip_chain(base: &Image, opts: &MipOptions) -> Result<Vec<Image>> {
    let mut chain = vec![base.clone()];
    let max_levels = full_mip_count(base.width, base.height);
    let levels = if opts.levels <= 0 { max_levels } else { opts.levels.min(max_levels) };
    let target = if opts.coverage_ref > 0.0 { alpha_coverage(base, opts.coverage_ref) } else { 0.0 };
    for i in 1..levels {
        let w = (base.width >> i).max(1);
        let h = (base.height >> i).max(1);
        if w.max(h) < opts.min_size {
            break;
        }
        let mut level = resample(base, w, h, &opts.resample)?;
        if opts.coverage_ref > 0.0 {
            scale_alpha_to_coverage(&mut level, opts.coverage_ref, target);
        }
        chain.push(level);
    }
    Ok(chain)
}

/// Fraction of pixels whose alpha is >= ref.
pub fn alpha_coverage(img: &Image, reference: f32) -> f32 {
    if img.is_empty() {
        return 0.0;
    }
    let covered = img.data.chunks_exact(4).filter(|p| p[3] >= reference).count();
    covered as f32 / img.pixel_count() as f32
}

/// Scales alpha so that alpha_coverage(img, ref) approximates `target`.
/// Keeps alpha-tested foliage/grates from thinning out in smaller mips.
pub fn scale_alpha_to_coverage(img: &mut Image, reference: f32, target: f32) {
    if img.is_empty() {
        return;
    }
    // Binary search for an alpha scale; coverage grows monotonically with it.
    let (mut lo, mut hi, mut best, mut best_err) = (0.0f32, 8.0f32, 1.0f32, 2.0f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        let covered = img.data.chunks_exact(4).filter(|p| p[3] * mid >= reference).count();
        let cov = covered as f32 / img.pixel_count() as f32;
        let err = if cov > target { cov - target } else { target - cov };
        if err < best_err {
            best_err = err;
            best = mid;
        }
        if cov < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    for p in img.data.chunks_exact_mut(4) {
        p[3] = clamp01(p[3] * best);
    }
}
