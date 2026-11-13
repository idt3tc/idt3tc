#include "img/resample.h"

#include "common/parallel.h"
#include "common/util.h"

#include <algorithm>
#include <cmath>

namespace idt3 {
namespace img {

namespace {

const double kPi = 3.14159265358979323846;

struct FilterDef {
    Filter id;
    const char* name;
    double support; // radius in source pixels at scale 1
};

const FilterDef kFilters[] = {
    {Filter::Nearest, "nearest", 0.5},   {Filter::Box, "box", 0.5},
    {Filter::Triangle, "triangle", 1.0}, {Filter::Hermite, "hermite", 1.0},
    {Filter::BSpline, "bspline", 2.0},   {Filter::Mitchell, "mitchell", 2.0},
    {Filter::CatmullRom, "catmullrom", 2.0}, {Filter::Gaussian, "gaussian", 1.5},
    {Filter::Lanczos2, "lanczos2", 2.0}, {Filter::Lanczos3, "lanczos3", 3.0},
    {Filter::Kaiser, "kaiser", 3.0},
};

const FilterDef& filterDef(Filter f)
{
    for (size_t i = 0; i < sizeof(kFilters) / sizeof(kFilters[0]); ++i) {
        if (kFilters[i].id == f)
            return kFilters[i];
    }
    return kFilters[0];
}

double sinc(double x)
{
    if (std::fabs(x) < 1e-8)
        return 1.0;
    x *= kPi;
    return std::sin(x) / x;
}

// Generic cubic from Mitchell & Netravali.
double cubicBC(double x, double B, double C)
{
    x = std::fabs(x);
    if (x < 1.0)
        return ((12 - 9 * B - 6 * C) * x * x * x + (-18 + 12 * B + 6 * C) * x * x + (6 - 2 * B)) / 6.0;
    if (x < 2.0)
        return ((-B - 6 * C) * x * x * x + (6 * B + 30 * C) * x * x + (-12 * B - 48 * C) * x + (8 * B + 24 * C)) /
               6.0;
    return 0.0;
}

// Zeroth order modified Bessel function of the first kind (series).
double besselI0(double x)
{
    double sum = 1.0, term = 1.0, q = x * x / 4.0;
    for (int k = 1; k < 50; ++k) {
        term *= q / (static_cast<double>(k) * k);
        sum += term;
        if (term < sum * 1e-12)
            break;
    }
    return sum;
}

double evalFilter(Filter f, double x)
{
    double ax = std::fabs(x);
    switch (f) {
    case Filter::Nearest:
    case Filter::Box:
        // Half-open so that integer downscales average exactly k samples.
        return (x >= -0.5 && x < 0.5) ? 1.0 : 0.0;
    case Filter::Triangle:
        return ax < 1.0 ? 1.0 - ax : 0.0;
    case Filter::Hermite:
        return ax < 1.0 ? (2.0 * ax - 3.0) * ax * ax + 1.0 : 0.0;
    case Filter::BSpline:
        return cubicBC(x, 1.0, 0.0);
    case Filter::Mitchell:
        return cubicBC(x, 1.0 / 3.0, 1.0 / 3.0);
    case Filter::CatmullRom:
        return cubicBC(x, 0.0, 0.5);
    case Filter::Gaussian:
        return ax < 1.5 ? std::exp(-2.0 * x * x) : 0.0;
    case Filter::Lanczos2:
        return ax < 2.0 ? sinc(x) * sinc(x / 2.0) : 0.0;
    case Filter::Lanczos3:
        return ax < 3.0 ? sinc(x) * sinc(x / 3.0) : 0.0;
    case Filter::Kaiser: {
        if (ax >= 3.0)
            return 0.0;
        const double alpha = 4.0;
        double r = x / 3.0;
        return sinc(x) * besselI0(alpha * std::sqrt(std::max(0.0, 1.0 - r * r))) / besselI0(alpha);
    }
    }
    return 0.0;
}

// Contribution of source samples to one destination sample. A source index
// of -1 stands for the fill color (EdgeMode::Fill outside the image).
struct Contrib {
    std::vector<int> index;
    std::vector<float> weight;
};

std::vector<Contrib> computeContribs(int srcSize, int dstSize, const ResampleOptions& o)
{
    std::vector<Contrib> out(dstSize);
    const double scale = static_cast<double>(dstSize) / srcSize;
    const FilterDef& def = filterDef(o.filter);

    if (o.filter == Filter::Nearest) {
        for (int i = 0; i < dstSize; ++i) {
            int s = static_cast<int>(std::floor((i + 0.5) / scale));
            out[i].index.push_back(std::min(srcSize - 1, std::max(0, s)));
            out[i].weight.push_back(1.0f);
        }
        return out;
    }

    // When minifying, stretch the filter so it covers all source pixels
    // that fall into a destination pixel (avoids aliasing).
    const double filterScale = std::max(1.0, 1.0 / scale) * std::max(0.01f, o.blur);
    const double support = def.support * filterScale;

    for (int i = 0; i < dstSize; ++i) {
        const double center = (i + 0.5) / scale - 0.5;
        int first = static_cast<int>(std::ceil(center - support));
        int last = static_cast<int>(std::floor(center + support));
        double total = 0.0;
        Contrib& c = out[i];
        for (int s = first; s <= last; ++s) {
            double w = evalFilter(o.filter, (s - center) / filterScale);
            if (w == 0.0)
                continue;
            c.index.push_back(resolveEdge(s, srcSize, o.edge));
            c.weight.push_back(static_cast<float>(w));
            total += w;
        }
        if (c.index.empty() || std::fabs(total) < 1e-12) {
            // Degenerate kernel (can happen with tiny blur values); fall
            // back to the nearest sample.
            c.index.assign(1, std::min(srcSize - 1, std::max(0, static_cast<int>(std::floor(center + 0.5)))));
            c.weight.assign(1, 1.0f);
            continue;
        }
        for (size_t k = 0; k < c.weight.size(); ++k)
            c.weight[k] = static_cast<float>(c.weight[k] / total);
    }
    return out;
}

// Applies horizontal then vertical contributions.
Image convolveSeparable(const Image& src, const std::vector<Contrib>& cx, const std::vector<Contrib>& cy,
                        const Color& fill, int jobs)
{
    const int dw = static_cast<int>(cx.size());
    const int dh = static_cast<int>(cy.size());
    const float fillv[4] = {fill.r, fill.g, fill.b, fill.a};

    // Horizontal pass: src.height rows of dw pixels.
    Image tmp(dw, src.height);
    parallelFor(src.height, jobs, [&](int y0, int y1) {
        for (int y = y0; y < y1; ++y) {
            const float* srow = src.row(y);
            float* drow = tmp.row(y);
            for (int x = 0; x < dw; ++x) {
                const Contrib& c = cx[x];
                float acc[4] = {0, 0, 0, 0};
                for (size_t k = 0; k < c.index.size(); ++k) {
                    const float w = c.weight[k];
                    const float* p = c.index[k] < 0 ? fillv : srow + c.index[k] * 4;
                    acc[0] += p[0] * w;
                    acc[1] += p[1] * w;
                    acc[2] += p[2] * w;
                    acc[3] += p[3] * w;
                }
                float* d = drow + x * 4;
                d[0] = acc[0];
                d[1] = acc[1];
                d[2] = acc[2];
                d[3] = acc[3];
            }
        }
    });

    // Vertical pass.
    Image dst(dw, dh);
    parallelFor(dh, jobs, [&](int y0, int y1) {
        std::vector<float> acc(static_cast<size_t>(dw) * 4);
        for (int y = y0; y < y1; ++y) {
            std::fill(acc.begin(), acc.end(), 0.0f);
            const Contrib& c = cy[y];
            for (size_t k = 0; k < c.index.size(); ++k) {
                const float w = c.weight[k];
                if (c.index[k] < 0) {
                    for (int x = 0; x < dw; ++x) {
                        for (int ch = 0; ch < 4; ++ch)
                            acc[x * 4 + ch] += fillv[ch] * w;
                    }
                    continue;
                }
                const float* srow = tmp.row(c.index[k]);
                for (size_t i = 0; i < acc.size(); ++i)
                    acc[i] += srow[i] * w;
            }
            std::copy(acc.begin(), acc.end(), dst.row(y));
        }
    });
    return dst;
}

// Shared pre/post processing around a separable filter.
Image filterImage(const Image& src, const std::vector<Contrib>& cx, const std::vector<Contrib>& cy, Color fill,
                  bool linear, bool premul, int jobs)
{
    Image work = src;
    if (linear) {
        toLinear(work);
        fill.r = srgbToLinear(fill.r);
        fill.g = srgbToLinear(fill.g);
        fill.b = srgbToLinear(fill.b);
    }
    if (premul) {
        premultiply(work);
        fill.r *= fill.a;
        fill.g *= fill.a;
        fill.b *= fill.a;
    }
    Image dst = convolveSeparable(work, cx, cy, fill, jobs);
    // Negative lobes (Lanczos, Catmull-Rom, ...) may overshoot.
    clampImage(dst);
    if (premul)
        unpremultiply(dst);
    if (linear)
        toSrgb(dst);
    return dst;
}

} // namespace

bool parseFilter(const std::string& name, Filter& out)
{
    std::string n = toLower(name);
    // Common aliases from other tools.
    if (n == "point")
        n = "nearest";
    else if (n == "bilinear" || n == "linear" || n == "tent")
        n = "triangle";
    else if (n == "bicubic" || n == "cubic")
        n = "catmullrom";
    else if (n == "lanczos")
        n = "lanczos3";
    for (size_t i = 0; i < sizeof(kFilters) / sizeof(kFilters[0]); ++i) {
        if (n == kFilters[i].name) {
            out = kFilters[i].id;
            return true;
        }
    }
    return false;
}

std::string filterName(Filter f)
{
    return filterDef(f).name;
}

std::vector<std::string> filterNames()
{
    std::vector<std::string> names;
    for (size_t i = 0; i < sizeof(kFilters) / sizeof(kFilters[0]); ++i)
        names.push_back(kFilters[i].name);
    return names;
}

Image resample(const Image& src, int w, int h, const ResampleOptions& opts, int jobs)
{
    checkDimensions(w, h, "resample");
    if (src.empty())
        throw Error("resample: empty source image");
    if (w == src.width && h == src.height && (opts.filter == Filter::Nearest || opts.blur == 1.0f) &&
        (opts.filter == Filter::Nearest || opts.filter == Filter::Box || opts.filter == Filter::Triangle ||
         opts.filter == Filter::Hermite || opts.filter == Filter::Lanczos2 || opts.filter == Filter::Lanczos3 ||
         opts.filter == Filter::CatmullRom || opts.filter == Filter::Kaiser))
        return src; // interpolating filters at scale 1 are the identity
    std::vector<Contrib> cx = computeContribs(src.width, w, opts);
    std::vector<Contrib> cy = computeContribs(src.height, h, opts);
    return filterImage(src, cx, cy, opts.fill, opts.linear, opts.premultiply && opts.filter != Filter::Nearest,
                       jobs);
}

Image gaussianBlur(const Image& src, float sigma, EdgeMode edge, const Color& fill, bool premultiplyAlpha,
                   int jobs)
{
    if (sigma <= 0.0f)
        return src;
    const int radius = std::max(1, static_cast<int>(std::ceil(sigma * 3.0f)));
    std::vector<float> kernel(2 * radius + 1);
    double total = 0.0;
    for (int i = -radius; i <= radius; ++i) {
        double w = std::exp(-(static_cast<double>(i) * i) / (2.0 * sigma * sigma));
        kernel[i + radius] = static_cast<float>(w);
        total += w;
    }
    for (size_t i = 0; i < kernel.size(); ++i)
        kernel[i] = static_cast<float>(kernel[i] / total);

    auto build = [&](int n) {
        std::vector<Contrib> cs(n);
        for (int i = 0; i < n; ++i) {
            for (int k = -radius; k <= radius; ++k) {
                cs[i].index.push_back(resolveEdge(i + k, n, edge));
                cs[i].weight.push_back(kernel[k + radius]);
            }
        }
        return cs;
    };
    return filterImage(src, build(src.width), build(src.height), fill, false, premultiplyAlpha, jobs);
}

} // namespace img
} // namespace idt3
