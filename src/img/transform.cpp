#include "img/transform.h"

#include "common/parallel.h"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace idt3 {
namespace img {

namespace {

const double kPi = 3.14159265358979323846;

void copyPixel(float* dst, const float* src)
{
    std::memcpy(dst, src, 4 * sizeof(float));
}

// Inverse-maps every destination pixel center through `map` and samples
// the (optionally premultiplied) source.
template <class MapFn>
Image warp(const Image& src, int w, int h, const AffineOptions& opts, int jobs, MapFn map)
{
    Image work = src;
    Color fill = opts.fill;
    const bool premul = opts.premultiply && opts.interp != Interp::Nearest;
    if (premul) {
        premultiply(work);
        fill.r *= fill.a;
        fill.g *= fill.a;
        fill.b *= fill.a;
    }
    Image dst(w, h);
    parallelFor(h, jobs, [&](int y0, int y1) {
        for (int y = y0; y < y1; ++y) {
            for (int x = 0; x < w; ++x) {
                double sx, sy;
                map(x + 0.5, y + 0.5, sx, sy);
                samplePixel(work, static_cast<float>(sx), static_cast<float>(sy), opts.interp, opts.edge, fill,
                            dst.pixel(x, y));
            }
        }
    });
    clampImage(dst);
    if (premul)
        unpremultiply(dst);
    return dst;
}

} // namespace

Image extractRegion(const Image& src, int x, int y, int w, int h, EdgeMode edge, const Color& fill)
{
    checkDimensions(w, h, "region");
    Image dst(w, h);
    for (int j = 0; j < h; ++j) {
        for (int i = 0; i < w; ++i)
            fetchPixel(src, x + i, y + j, edge, fill, dst.pixel(i, j));
    }
    return dst;
}

Image rotateQuarter(const Image& src, int quarters)
{
    quarters = ((quarters % 4) + 4) % 4;
    if (quarters == 0)
        return src;
    if (quarters == 2) {
        Image dst(src.width, src.height);
        for (int y = 0; y < src.height; ++y) {
            for (int x = 0; x < src.width; ++x)
                copyPixel(dst.pixel(src.width - 1 - x, src.height - 1 - y), src.pixel(x, y));
        }
        return dst;
    }
    Image dst(src.height, src.width);
    for (int y = 0; y < src.height; ++y) {
        for (int x = 0; x < src.width; ++x) {
            if (quarters == 1) // clockwise: (x, y) -> (h-1-y, x)
                copyPixel(dst.pixel(src.height - 1 - y, x), src.pixel(x, y));
            else // counter-clockwise: (x, y) -> (y, w-1-x)
                copyPixel(dst.pixel(y, src.width - 1 - x), src.pixel(x, y));
        }
    }
    return dst;
}

Image flipHorizontal(const Image& src)
{
    Image dst(src.width, src.height);
    for (int y = 0; y < src.height; ++y) {
        for (int x = 0; x < src.width; ++x)
            copyPixel(dst.pixel(src.width - 1 - x, y), src.pixel(x, y));
    }
    return dst;
}

Image flipVertical(const Image& src)
{
    Image dst(src.width, src.height);
    for (int y = 0; y < src.height; ++y)
        std::memcpy(dst.row(src.height - 1 - y), src.row(y), sizeof(float) * 4 * src.width);
    return dst;
}

Image transpose(const Image& src)
{
    Image dst(src.height, src.width);
    for (int y = 0; y < src.height; ++y) {
        for (int x = 0; x < src.width; ++x)
            copyPixel(dst.pixel(y, x), src.pixel(x, y));
    }
    return dst;
}

Image rotateArbitrary(const Image& src, double degrees, bool expand, const AffineOptions& opts, int jobs)
{
    const double rad = degrees * kPi / 180.0;
    const double c = std::cos(rad), s = std::sin(rad);
    int w = src.width, h = src.height;
    if (expand) {
        // Bounding box of the rotated rectangle; the epsilon avoids growing
        // by a pixel due to floating point noise.
        double bw = std::fabs(src.width * c) + std::fabs(src.height * s);
        double bh = std::fabs(src.width * s) + std::fabs(src.height * c);
        w = std::max(1, static_cast<int>(std::ceil(bw - 1e-6)));
        h = std::max(1, static_cast<int>(std::ceil(bh - 1e-6)));
    }
    checkDimensions(w, h, "rotate");
    const double scx = src.width / 2.0, scy = src.height / 2.0;
    const double dcx = w / 2.0, dcy = h / 2.0;
    // With y pointing down, a clockwise rotation on screen is the standard
    // rotation matrix; its inverse maps destination back to source.
    return warp(src, w, h, opts, jobs, [&](double x, double y, double& sx, double& sy) {
        double rx = x - dcx, ry = y - dcy;
        sx = c * rx + s * ry + scx;
        sy = -s * rx + c * ry + scy;
    });
}

Image translate(const Image& src, double dx, double dy, const AffineOptions& opts, int jobs)
{
    // Integral shifts are pure index remapping; avoid filtering entirely.
    if (dx == std::floor(dx) && dy == std::floor(dy)) {
        Image dst(src.width, src.height);
        const int ix = static_cast<int>(dx), iy = static_cast<int>(dy);
        for (int y = 0; y < src.height; ++y) {
            for (int x = 0; x < src.width; ++x)
                fetchPixel(src, x - ix, y - iy, opts.edge, opts.fill, dst.pixel(x, y));
        }
        return dst;
    }
    return warp(src, src.width, src.height, opts, jobs, [&](double x, double y, double& sx, double& sy) {
        sx = x - dx;
        sy = y - dy;
    });
}

} // namespace img
} // namespace idt3
