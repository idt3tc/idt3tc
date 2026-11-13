#include "img/sampling.h"

#include <cmath>

namespace idt3 {
namespace img {

int resolveEdge(int i, int n, EdgeMode mode)
{
    if (i >= 0 && i < n)
        return i;
    switch (mode) {
    case EdgeMode::Clamp:
        return i < 0 ? 0 : n - 1;
    case EdgeMode::Repeat: {
        int m = i % n;
        return m < 0 ? m + n : m;
    }
    case EdgeMode::Mirror: {
        // Period of 2n: 0..n-1 forward, then n-1..0 backwards.
        int period = 2 * n;
        int m = i % period;
        if (m < 0)
            m += period;
        return m < n ? m : period - 1 - m;
    }
    case EdgeMode::Fill:
    default:
        return -1;
    }
}

void fetchPixel(const Image& img, int x, int y, EdgeMode edge, const Color& fill, float out[4])
{
    int sx = resolveEdge(x, img.width, edge);
    int sy = resolveEdge(y, img.height, edge);
    if (sx < 0 || sy < 0) {
        out[0] = fill.r;
        out[1] = fill.g;
        out[2] = fill.b;
        out[3] = fill.a;
        return;
    }
    const float* p = img.pixel(sx, sy);
    out[0] = p[0];
    out[1] = p[1];
    out[2] = p[2];
    out[3] = p[3];
}

namespace {

// Catmull-Rom spline weights for fractional offset t in [0, 1).
void catmullRomWeights(float t, float w[4])
{
    float t2 = t * t;
    float t3 = t2 * t;
    w[0] = 0.5f * (-t3 + 2.0f * t2 - t);
    w[1] = 0.5f * (3.0f * t3 - 5.0f * t2 + 2.0f);
    w[2] = 0.5f * (-3.0f * t3 + 4.0f * t2 + t);
    w[3] = 0.5f * (t3 - t2);
}

} // namespace

void samplePixel(const Image& img, float x, float y, Interp interp, EdgeMode edge, const Color& fill,
                 float out[4])
{
    if (interp == Interp::Nearest) {
        fetchPixel(img, static_cast<int>(std::floor(x)), static_cast<int>(std::floor(y)), edge, fill, out);
        return;
    }

    // Shift to pixel-center coordinates.
    float fx = x - 0.5f;
    float fy = y - 0.5f;
    int ix = static_cast<int>(std::floor(fx));
    int iy = static_cast<int>(std::floor(fy));
    float tx = fx - ix;
    float ty = fy - iy;

    float acc[4] = {0, 0, 0, 0};
    float p[4];
    if (interp == Interp::Bilinear) {
        const float wx[2] = {1.0f - tx, tx};
        const float wy[2] = {1.0f - ty, ty};
        for (int j = 0; j < 2; ++j) {
            for (int i = 0; i < 2; ++i) {
                float w = wx[i] * wy[j];
                if (w == 0.0f)
                    continue;
                fetchPixel(img, ix + i, iy + j, edge, fill, p);
                for (int c = 0; c < 4; ++c)
                    acc[c] += p[c] * w;
            }
        }
    } else {
        float wx[4], wy[4];
        catmullRomWeights(tx, wx);
        catmullRomWeights(ty, wy);
        for (int j = 0; j < 4; ++j) {
            for (int i = 0; i < 4; ++i) {
                float w = wx[i] * wy[j];
                if (w == 0.0f)
                    continue;
                fetchPixel(img, ix - 1 + i, iy - 1 + j, edge, fill, p);
                for (int c = 0; c < 4; ++c)
                    acc[c] += p[c] * w;
            }
        }
    }
    for (int c = 0; c < 4; ++c)
        out[c] = acc[c];
}

} // namespace img
} // namespace idt3
