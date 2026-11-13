#pragma once

#include "img/image.h"
#include "img/resample.h"

#include <vector>

namespace idt3 {
namespace img {

struct MipOptions {
    int levels;      // total levels incl. base; 0 = full chain down to 1x1
    int minSize;     // stop before the larger dimension drops below this
    ResampleOptions resample;
    float coverageRef; // >0: preserve alpha-test coverage at this alpha ref

    MipOptions() : levels(0), minSize(1), coverageRef(0.0f)
    {
        resample.filter = Filter::Box;
        // Textures in idTech 3 almost always tile.
        resample.edge = EdgeMode::Repeat;
    }
};

// Number of levels of a full chain for w x h (down to 1x1).
int fullMipCount(int w, int h);

// Returns base + generated levels. Every level is filtered directly from
// the base image (no cumulative blur from repeated halving).
std::vector<Image> buildMipChain(const Image& base, const MipOptions& opts, int jobs);

// Fraction of pixels whose alpha is >= ref.
float alphaCoverage(const Image& img, float ref);

// Scales alpha so that alphaCoverage(img, ref) approximates `target`.
// Keeps alpha-tested foliage/grates from thinning out in smaller mips.
void scaleAlphaToCoverage(Image& img, float ref, float target);

} // namespace img
} // namespace idt3
