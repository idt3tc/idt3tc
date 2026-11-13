#include "img/mipmap.h"

#include <algorithm>

namespace idt3 {
namespace img {

int fullMipCount(int w, int h)
{
    int n = 1;
    int m = std::max(w, h);
    while (m > 1) {
        m >>= 1;
        ++n;
    }
    return n;
}

std::vector<Image> buildMipChain(const Image& base, const MipOptions& opts, int jobs)
{
    std::vector<Image> chain;
    chain.push_back(base);
    int maxLevels = fullMipCount(base.width, base.height);
    int levels = opts.levels <= 0 ? maxLevels : std::min(opts.levels, maxLevels);
    const float targetCoverage = opts.coverageRef > 0.0f ? alphaCoverage(base, opts.coverageRef) : 0.0f;

    for (int i = 1; i < levels; ++i) {
        int w = std::max(1, base.width >> i);
        int h = std::max(1, base.height >> i);
        if (std::max(w, h) < opts.minSize)
            break;
        Image level = resample(base, w, h, opts.resample, jobs);
        if (opts.coverageRef > 0.0f)
            scaleAlphaToCoverage(level, opts.coverageRef, targetCoverage);
        chain.push_back(level);
    }
    return chain;
}

float alphaCoverage(const Image& img, float ref)
{
    if (img.empty())
        return 0.0f;
    size_t covered = 0;
    for (size_t i = 3; i < img.data.size(); i += 4) {
        if (img.data[i] >= ref)
            ++covered;
    }
    return static_cast<float>(covered) / static_cast<float>(img.pixelCount());
}

void scaleAlphaToCoverage(Image& img, float ref, float target)
{
    // Binary search for an alpha scale; coverage grows monotonically with it.
    float lo = 0.0f, hi = 8.0f, best = 1.0f, bestErr = 2.0f;
    for (int iter = 0; iter < 24; ++iter) {
        float mid = 0.5f * (lo + hi);
        size_t covered = 0;
        for (size_t i = 3; i < img.data.size(); i += 4) {
            if (img.data[i] * mid >= ref)
                ++covered;
        }
        float cov = static_cast<float>(covered) / static_cast<float>(img.pixelCount());
        float err = cov > target ? cov - target : target - cov;
        if (err < bestErr) {
            bestErr = err;
            best = mid;
        }
        if (cov < target)
            lo = mid;
        else
            hi = mid;
    }
    for (size_t i = 3; i < img.data.size(); i += 4)
        img.data[i] = clamp01(img.data[i] * best);
}

} // namespace img
} // namespace idt3
