#pragma once

#include "img/image.h"
#include "img/sampling.h"

namespace idt3 {
namespace img {

// Copies the w x h region at (x, y); parts outside the source are resolved
// through `edge` (fill, clamp, repeat or mirror).
Image extractRegion(const Image& src, int x, int y, int w, int h, EdgeMode edge, const Color& fill);

// Lossless quarter turns; `quarters` is taken modulo 4, clockwise.
Image rotateQuarter(const Image& src, int quarters);
Image flipHorizontal(const Image& src);
Image flipVertical(const Image& src);
Image transpose(const Image& src);

struct AffineOptions {
    Interp interp;
    EdgeMode edge;
    Color fill;
    bool premultiply;
    AffineOptions() : interp(Interp::Bicubic), edge(EdgeMode::Fill), premultiply(true) {}
};

// Rotates by `degrees` clockwise around the image center. With `expand`
// the canvas grows to contain the whole rotated image.
Image rotateArbitrary(const Image& src, double degrees, bool expand, const AffineOptions& opts, int jobs);

// Shifts content by (dx, dy) pixels (fractional allowed); with
// EdgeMode::Repeat this is the classic texture "offset" for checking seams.
Image translate(const Image& src, double dx, double dy, const AffineOptions& opts, int jobs);

} // namespace img
} // namespace idt3
