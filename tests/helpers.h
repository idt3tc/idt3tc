#pragma once

// Shared test fixtures: synthetic images and comparison metrics.

#include "img/image.h"

#include <cmath>

namespace idt3test {

// Smooth color gradients with a soft alpha ramp and some detail, so block
// compressors have realistic content to work with.
inline idt3::img::Image makeTestImage(int w, int h, bool withAlpha)
{
    idt3::img::Image img(w, h);
    for (int y = 0; y < h; ++y) {
        for (int x = 0; x < w; ++x) {
            float* p = img.pixel(x, y);
            float u = (x + 0.5f) / w, v = (y + 0.5f) / h;
            p[0] = u;
            p[1] = v;
            p[2] = 0.5f + 0.5f * std::sin(6.2831853f * (u + v));
            p[3] = withAlpha ? 0.25f + 0.75f * u * v : 1.0f;
        }
    }
    return img;
}

// Peak signal-to-noise ratio (dB) over the channels in `mask` (bit 0 = r)
// after 8-bit quantization of both images.
inline double psnr(const idt3::img::Image& a, const idt3::img::Image& b, int mask)
{
    double se = 0;
    size_t n = 0;
    for (size_t i = 0; i < a.data.size(); i += 4) {
        for (int c = 0; c < 4; ++c) {
            if (!(mask & (1 << c)))
                continue;
            double d = double(idt3::img::toByte(a.data[i + c])) - idt3::img::toByte(b.data[i + c]);
            se += d * d;
            ++n;
        }
    }
    if (se == 0)
        return 99.0;
    return 10.0 * std::log10(255.0 * 255.0 / (se / n));
}

// Largest per-channel difference in 8-bit steps.
inline int maxByteDiff(const idt3::img::Image& a, const idt3::img::Image& b)
{
    int m = 0;
    for (size_t i = 0; i < a.data.size(); ++i) {
        int d = std::abs(int(idt3::img::toByte(a.data[i])) - int(idt3::img::toByte(b.data[i])));
        if (d > m)
            m = d;
    }
    return m;
}

} // namespace idt3test
