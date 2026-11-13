#pragma once

#include "img/image.h"
#include "img/sampling.h"

#include <string>
#include <vector>

namespace idt3 {
namespace img {

// Reconstruction filters for resampling.
enum class Filter {
    Nearest,
    Box,
    Triangle,   // a.k.a. bilinear / tent
    Hermite,
    BSpline,    // cubic B-spline (B=1, C=0), very smooth
    Mitchell,   // Mitchell-Netravali (B=1/3, C=1/3)
    CatmullRom, // B=0, C=1/2, sharper bicubic
    Gaussian,
    Lanczos2,
    Lanczos3,
    Kaiser      // windowed sinc with a Kaiser window (radius 3)
};

// Name <-> enum for the command line and help output.
bool parseFilter(const std::string& name, Filter& out);
std::string filterName(Filter f);
std::vector<std::string> filterNames();

struct ResampleOptions {
    Filter filter;
    EdgeMode edge;
    Color fill;       // used with EdgeMode::Fill
    bool linear;      // filter in linear light (sRGB decode/encode around it)
    bool premultiply; // weight colors by alpha to avoid dark/colored fringes
    float blur;       // >1 widens the filter (softer), <1 narrows it (sharper)

    ResampleOptions()
        : filter(Filter::Mitchell), edge(EdgeMode::Clamp), linear(false), premultiply(true), blur(1.0f)
    {
    }
};

// Resamples `src` to w x h with separable filtering.
Image resample(const Image& src, int w, int h, const ResampleOptions& opts, int jobs);

// Separable Gaussian convolution with standard deviation `sigma` (pixels).
// Alpha-weighted when `premultiplyAlpha` is set.
Image gaussianBlur(const Image& src, float sigma, EdgeMode edge, const Color& fill, bool premultiplyAlpha,
                   int jobs);

} // namespace img
} // namespace idt3
