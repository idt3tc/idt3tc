#pragma once

#include "img/image.h"

namespace idt3 {
namespace img {

// How coordinates outside the image are resolved.
enum class EdgeMode {
    Clamp,  // repeat the outermost pixels
    Repeat, // wrap around (tiling textures)
    Mirror, // reflect at the border
    Fill    // use a constant fill color
};

// Maps an integer coordinate into [0, n) according to `mode`. Returns -1 for
// EdgeMode::Fill when `i` is outside the image.
int resolveEdge(int i, int n, EdgeMode mode);

// Interpolation used for arbitrary (sub-pixel) sampling.
enum class Interp { Nearest, Bilinear, Bicubic };

// Reads a pixel honoring the edge mode.
void fetchPixel(const Image& img, int x, int y, EdgeMode edge, const Color& fill, float out[4]);

// Samples `img` at continuous coordinates where pixel (i, j) covers
// [i, i+1) x [j, j+1), i.e. its center is at (i + 0.5, j + 0.5).
void samplePixel(const Image& img, float x, float y, Interp interp, EdgeMode edge, const Color& fill,
                 float out[4]);

} // namespace img
} // namespace idt3
