// Per-pixel color, alpha and filter operations.

#include "common/util.h"
#include "img/op_util.h"
#include "img/mipmap.h"
#include "img/resample.h"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace idt3 {
namespace img {

namespace {

const char* const kChannelsHelp = "channels to modify, any of r, g, b, a";

double parseNumberArg(const std::string& op, const std::string& arg)
{
    double v;
    if (!parseDouble(arg, v))
        throw UsageError(op + ": expected a number, got '" + arg + "'");
    return v;
}

Color parseColorArg(const std::string& op, const std::string& arg)
{
    Color c;
    if (!parseColor(arg, c))
        throw UsageError(op + ": invalid color '" + arg + "'");
    return c;
}

// Applies fn(value) to every channel selected by `mask`.
template <class Fn>
void mapChannels(Image& img, int mask, Fn fn)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        for (int c = 0; c < 4; ++c) {
            if (mask & (1 << c))
                img.data[i + c] = fn(img.data[i + c]);
        }
    }
}

// Ops that map each selected channel independently through a function
// built from the argument.
typedef std::function<float(float)> ChannelFn;

std::unique_ptr<Op> channelOp(const std::string& name, OptionReader& o, const ChannelFn& fn)
{
    int mask = o.channels("channels", 7);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage(name);
        mapChannels(img, mask, [&](float v) { return clamp01(fn(v)); });
    });
}

std::unique_ptr<Op> createGamma(const std::string& arg, OptionReader& o)
{
    double g = parseNumberArg("--gamma", arg);
    if (g <= 0)
        throw UsageError("--gamma: value must be > 0");
    const float inv = static_cast<float>(1.0 / g);
    return channelOp("--gamma", o, [inv](float v) { return v <= 0 ? 0.0f : std::pow(v, inv); });
}

std::unique_ptr<Op> createBrightness(const std::string& arg, OptionReader& o)
{
    const float d = static_cast<float>(parseNumberArg("--brightness", arg));
    return channelOp("--brightness", o, [d](float v) { return v + d; });
}

std::unique_ptr<Op> createContrast(const std::string& arg, OptionReader& o)
{
    const float f = static_cast<float>(parseNumberArg("--contrast", arg));
    const float pivot = static_cast<float>(o.number("pivot", 0.5, 0.0, 1.0));
    return channelOp("--contrast", o, [f, pivot](float v) { return (v - pivot) * f + pivot; });
}

std::unique_ptr<Op> createLevels(const std::string& arg, OptionReader& o)
{
    std::vector<std::string> parts = split(arg, ',');
    double v[5] = {0, 255, 1, 0, 255};
    if (parts.size() != 2 && parts.size() != 3 && parts.size() != 5)
        throw UsageError("--levels: expected IN_LO,IN_HI[,GAMMA[,OUT_LO,OUT_HI]]");
    for (size_t i = 0; i < parts.size(); ++i) {
        if (!parseDouble(parts[i], v[i]))
            throw UsageError("--levels: invalid number '" + parts[i] + "'");
    }
    if (v[1] <= v[0] || v[2] <= 0)
        throw UsageError("--levels: need IN_LO < IN_HI and GAMMA > 0");
    const float inLo = static_cast<float>(v[0] / 255), inHi = static_cast<float>(v[1] / 255);
    const float invGamma = static_cast<float>(1.0 / v[2]);
    const float outLo = static_cast<float>(v[3] / 255), outHi = static_cast<float>(v[4] / 255);
    return channelOp("--levels", o, [=](float c) {
        float t = clamp01((c - inLo) / (inHi - inLo));
        return outLo + std::pow(t, invGamma) * (outHi - outLo);
    });
}

std::unique_ptr<Op> createInvert(const std::string&, OptionReader& o)
{
    return channelOp("--invert", o, [](float v) { return 1.0f - v; });
}

std::unique_ptr<Op> createSaturation(const std::string& arg, OptionReader&)
{
    const float f = static_cast<float>(parseNumberArg("--saturation", arg));
    return makeOp([f](Context& ctx) {
        Image& img = ctx.requireImage("--saturation");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            float l = luma(p);
            for (int c = 0; c < 3; ++c)
                p[c] = clamp01(l + (p[c] - l) * f);
        }
    });
}

std::unique_ptr<Op> createHue(const std::string& arg, OptionReader&)
{
    const double deg = parseNumberArg("--hue", arg);
    // Rotation around the gray axis (1,1,1) of the RGB cube.
    const double a = deg * 3.14159265358979323846 / 180.0;
    const float cs = static_cast<float>(std::cos(a)), sn = static_cast<float>(std::sin(a));
    const float k = (1.0f - cs) / 3.0f, s3 = std::sqrt(1.0f / 3.0f) * sn;
    const float m[3][3] = {{cs + k, k - s3, k + s3}, {k + s3, cs + k, k - s3}, {k - s3, k + s3, cs + k}};
    std::vector<float> mat(&m[0][0], &m[0][0] + 9);
    return makeOp([mat](Context& ctx) {
        Image& img = ctx.requireImage("--hue");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            float r = p[0], g = p[1], b = p[2];
            for (int c = 0; c < 3; ++c)
                p[c] = clamp01(mat[c * 3] * r + mat[c * 3 + 1] * g + mat[c * 3 + 2] * b);
        }
    });
}

std::unique_ptr<Op> createGrayscale(const std::string&, OptionReader& o)
{
    int w = o.choice("weights", {"rec709", "rec601", "average", "max"}, 0);
    return makeOp([w](Context& ctx) {
        Image& img = ctx.requireImage("--grayscale");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            float v;
            switch (w) {
            case 0: v = luma(p); break;
            case 1: v = 0.299f * p[0] + 0.587f * p[1] + 0.114f * p[2]; break;
            case 2: v = (p[0] + p[1] + p[2]) / 3.0f; break;
            default: v = std::max(p[0], std::max(p[1], p[2])); break;
            }
            p[0] = p[1] = p[2] = clamp01(v);
        }
    });
}

std::unique_ptr<Op> createTint(const std::string& arg, OptionReader& o)
{
    Color c = parseColorArg("--tint", arg);
    int mask = o.channels("channels", 7);
    return makeOp([c, mask](Context& ctx) {
        Image& img = ctx.requireImage("--tint");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            for (int ch = 0; ch < 4; ++ch) {
                if (mask & (1 << ch))
                    img.data[i + ch] *= c[ch];
            }
        }
    });
}

std::unique_ptr<Op> createFill(const std::string& arg, OptionReader& o)
{
    Color c = parseColorArg("--fill", arg);
    int mask = o.channels("channels", 15);
    return makeOp([c, mask](Context& ctx) {
        Image& img = ctx.requireImage("--fill");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            for (int ch = 0; ch < 4; ++ch) {
                if (mask & (1 << ch))
                    img.data[i + ch] = c[ch];
            }
        }
    });
}

std::unique_ptr<Op> createColorKey(const std::string& arg, OptionReader& o)
{
    Color key = parseColorArg("--colorkey", arg);
    float fuzz = static_cast<float>(o.number("fuzz", 0.0, 0.0, 1.0));
    float soft = static_cast<float>(o.number("soft", 0.0, 0.0, 1.0));
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--colorkey");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            float d = std::max(std::fabs(p[0] - key.r), std::max(std::fabs(p[1] - key.g), std::fabs(p[2] - key.b)));
            if (d <= fuzz + 1e-6f)
                p[3] = 0.0f;
            else if (soft > 0 && d < fuzz + soft)
                p[3] *= (d - fuzz) / soft;
        }
    });
}

std::unique_ptr<Op> createAlpha(const std::string& arg, OptionReader& o)
{
    const std::vector<std::string> modes = {"opaque",   "clear",       "set",           "scale", "invert",
                                            "luma",     "threshold",   "premultiply",   "unpremultiply",
                                            "coverage"};
    std::string m = toLower(arg);
    int mode = -1;
    for (size_t i = 0; i < modes.size(); ++i) {
        if (m == modes[i])
            mode = static_cast<int>(i);
    }
    if (mode < 0)
        throw UsageError("--alpha: unknown mode '" + arg + "' (expected " + join(modes, ", ") + ")");

    float value = 1.0f, ref = 0.5f, target = -1.0f;
    if (m == "set" || m == "scale") {
        if (!o.has("value"))
            throw UsageError("--alpha " + m + ": needs value=...");
        value = static_cast<float>(o.number("value", 1.0, 0.0, m == "set" ? 1.0 : 100.0));
    }
    if (m == "threshold" || m == "coverage")
        ref = static_cast<float>(o.number("ref", 0.5, 0.0, 1.0));
    if (m == "coverage")
        target = static_cast<float>(o.number("target", 0.5, 0.0, 1.0));

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--alpha");
        if (mode == 7) {
            premultiply(img);
            return;
        }
        if (mode == 8) {
            unpremultiply(img);
            return;
        }
        if (mode == 9) {
            scaleAlphaToCoverage(img, ref, target);
            return;
        }
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            switch (mode) {
            case 0: p[3] = 1.0f; break;
            case 1: p[3] = 0.0f; break;
            case 2: p[3] = value; break;
            case 3: p[3] = clamp01(p[3] * value); break;
            case 4: p[3] = 1.0f - p[3]; break;
            case 5: p[3] = clamp01(luma(p)); break;
            case 6: p[3] = p[3] >= ref ? 1.0f : 0.0f; break;
            }
        }
    });
}

std::unique_ptr<Op> createFlatten(const std::string&, OptionReader& o)
{
    Color bg = o.color("bg", Color(0, 0, 0, 1));
    return makeOp([bg](Context& ctx) {
        Image& img = ctx.requireImage("--flatten");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float* p = &img.data[i];
            float a = p[3];
            for (int c = 0; c < 3; ++c)
                p[c] = p[c] * a + bg[c] * (1.0f - a);
            p[3] = 1.0f;
        }
    });
}

std::unique_ptr<Op> createSwizzle(const std::string& arg, OptionReader&)
{
    std::string s = toLower(arg);
    if (s.size() == 3)
        s += 'a';
    if (s.size() != 4)
        throw UsageError("--swizzle: expected 3 or 4 characters from r, g, b, a, 0, 1 (e.g. bgra, rrr1)");
    // Source index per destination channel; 4 = constant 0, 5 = constant 1.
    std::vector<int> src(4);
    for (int i = 0; i < 4; ++i) {
        const char* chars = "rgba01";
        const char* p = std::strchr(chars, s[i]);
        if (!p || !s[i])
            throw UsageError("--swizzle: invalid channel '" + std::string(1, s[i]) + "' in '" + arg + "'");
        src[i] = static_cast<int>(p - chars);
    }
    return makeOp([src](Context& ctx) {
        Image& img = ctx.requireImage("--swizzle");
        for (size_t i = 0; i < img.data.size(); i += 4) {
            float in[6] = {img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3], 0.0f, 1.0f};
            for (int c = 0; c < 4; ++c)
                img.data[i + c] = in[src[c]];
        }
    });
}

std::unique_ptr<Op> createBlur(const std::string& arg, OptionReader& o)
{
    double sigma = parseNumberArg("--blur", arg);
    if (sigma < 0 || sigma > 500)
        throw UsageError("--blur: sigma must be in [0, 500]");
    EdgeMode edge = o.edge("edge", EdgeMode::Clamp);
    Color bg = o.color("bg", Color());
    bool premul = o.flag("premultiply", true);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--blur");
        img = gaussianBlur(img, static_cast<float>(sigma), edge, bg, premul, ctx.jobs);
    });
}

std::unique_ptr<Op> createSharpen(const std::string& arg, OptionReader& o)
{
    const float amount = static_cast<float>(parseNumberArg("--sharpen", arg));
    const float sigma = static_cast<float>(o.number("sigma", 1.0, 0.1, 100.0));
    const float threshold = static_cast<float>(o.number("threshold", 0.0, 0.0, 1.0));
    EdgeMode edge = o.edge("edge", EdgeMode::Clamp);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--sharpen");
        // Unsharp mask on RGB; alpha is left untouched.
        Image blurred = gaussianBlur(img, sigma, edge, Color(), true, ctx.jobs);
        for (size_t i = 0; i < img.data.size(); i += 4) {
            for (int c = 0; c < 3; ++c) {
                float diff = img.data[i + c] - blurred.data[i + c];
                if (std::fabs(diff) > threshold)
                    img.data[i + c] = clamp01(img.data[i + c] + amount * diff);
            }
        }
    });
}

std::unique_ptr<Op> createNormalMap(const std::string&, OptionReader& o)
{
    const float strength = static_cast<float>(o.number("strength", 2.0, 0.0, 1000.0));
    const int source = o.choice("source", {"luma", "r", "g", "b", "a"}, 0);
    const EdgeMode edge = o.edge("edge", EdgeMode::Repeat);
    const bool flipX = o.flag("flip-x", false);
    const bool flipY = o.flag("flip-y", false);
    const bool heightAlpha = o.flag("height-alpha", false);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--normalmap");
        const int w = img.width, h = img.height;
        std::vector<float> height(img.pixelCount());
        for (size_t i = 0; i < height.size(); ++i) {
            const float* p = &img.data[i * 4];
            height[i] = source == 0 ? luma(p) : p[source - 1];
        }
        auto H = [&](int x, int y) {
            int sx = resolveEdge(x, w, edge == EdgeMode::Fill ? EdgeMode::Clamp : edge);
            int sy = resolveEdge(y, h, edge == EdgeMode::Fill ? EdgeMode::Clamp : edge);
            return height[static_cast<size_t>(sy) * w + sx];
        };
        Image out(w, h);
        for (int y = 0; y < h; ++y) {
            for (int x = 0; x < w; ++x) {
                // Sobel gradients (image y axis points down).
                float dx = (H(x + 1, y - 1) + 2 * H(x + 1, y) + H(x + 1, y + 1)) -
                           (H(x - 1, y - 1) + 2 * H(x - 1, y) + H(x - 1, y + 1));
                float dy = (H(x - 1, y + 1) + 2 * H(x, y + 1) + H(x + 1, y + 1)) -
                           (H(x - 1, y - 1) + 2 * H(x, y - 1) + H(x + 1, y - 1));
                // OpenGL convention: +Y (green) points up in the texture.
                float nx = -dx * strength / 8.0f * (flipX ? -1.0f : 1.0f);
                float ny = dy * strength / 8.0f * (flipY ? -1.0f : 1.0f);
                float nz = 1.0f;
                float len = std::sqrt(nx * nx + ny * ny + nz * nz);
                float* d = out.pixel(x, y);
                d[0] = nx / len * 0.5f + 0.5f;
                d[1] = ny / len * 0.5f + 0.5f;
                d[2] = nz / len * 0.5f + 0.5f;
                d[3] = heightAlpha ? height[static_cast<size_t>(y) * w + x] : 1.0f;
            }
        }
        img = out;
    });
}

} // namespace

void registerColorOps(OpRegistry& r)
{
    const std::string cat = "Color & alpha";
    std::vector<OptionHelp> rgbChannels = {opt("channels", "rgb", kChannelsHelp)};
    auto add = [&r](const std::string& name, const std::string& argName, const std::string& category,
                    const std::string& summary, const std::vector<OptionHelp>& options, OpFactory create) {
        OpInfo i;
        i.name = name;
        i.arg = argName;
        i.category = category;
        i.summary = summary;
        i.options = options;
        i.create = create;
        r.add(i);
    };

    add("gamma", "G", cat, "gamma correction: v = v^(1/G); G > 1 brightens midtones", rgbChannels, createGamma);
    add("brightness", "DELTA", cat, "add DELTA (-1..1) to every value", rgbChannels, createBrightness);
    add("contrast", "FACTOR", cat, "scale values around the pivot; FACTOR > 1 increases contrast",
        withOptions(rgbChannels, {opt("pivot", "0.5", "value that stays unchanged")}), createContrast);
    add("levels", "LO,HI[,GAMMA[,OUTLO,OUTHI]]", cat,
        "map input range LO..HI (0-255) to OUTLO..OUTHI with optional gamma", rgbChannels, createLevels);
    add("saturation", "FACTOR", cat, "0 = grayscale, 1 = unchanged, >1 more saturated", {}, createSaturation);
    add("hue", "DEGREES", cat, "rotate hues", {}, createHue);
    add("grayscale", "", cat, "convert to gray (keeps RGB layout)",
        {opt("weights", "rec709", "rec709, rec601, average or max")}, createGrayscale);
    add("invert", "", cat, "negate values", rgbChannels, createInvert);
    add("tint", "COLOR", cat, "multiply by a color", rgbChannels, createTint);
    add("fill", "COLOR", cat, "set channels to a constant color", {opt("channels", "rgba", kChannelsHelp)},
        createFill);
    add("colorkey", "COLOR", cat, "make pixels of COLOR transparent (e.g. #ff00ff sprite backgrounds)",
        {opt("fuzz", "0", "max. per-channel difference still matched (0..1)"),
         opt("soft", "0", "feather: alpha ramps up over this extra distance")},
        createColorKey);
    add("alpha", "MODE", cat,
        "alpha channel ops: opaque, clear, set, scale, invert, luma, threshold, premultiply, unpremultiply, "
        "coverage",
        {opt("value", "", "set: new alpha (0..1); scale: factor"),
         opt("ref", "0.5", "threshold/coverage: alpha reference (0.5 = idTech 3 alphaFunc GE128)"),
         opt("target", "0.5", "coverage: fraction of pixels that should pass the alpha test")},
        createAlpha);
    add("flatten", "", cat, "composite onto a solid background and drop alpha",
        {opt("bg", "black", "background color")}, createFlatten);
    add("swizzle", "PATTERN", cat,
        "reorder channels, e.g. bgra, rrr1, ggga (r, g, b, a, 0, 1; 3 chars keep alpha)", {}, createSwizzle);

    const std::string fcat = "Filters";
    add("blur", "SIGMA", fcat, "Gaussian blur with standard deviation SIGMA pixels",
        {opt("edge", "clamp", "clamp, repeat (tiling textures), mirror, fill"),
         opt("bg", "transparent", "color for edge=fill"), opt("premultiply", "1", "alpha-weighted blur")},
        createBlur);
    add("sharpen", "AMOUNT", fcat, "unsharp mask; AMOUNT around 0.5..2",
        {opt("sigma", "1", "blur radius of the mask"), opt("threshold", "0", "minimum difference to sharpen"),
         opt("edge", "clamp", "clamp, repeat, mirror")},
        createSharpen);
    add("normalmap", "", fcat, "turn a height map into a tangent-space normal map",
        {opt("strength", "2", "height scale (bumpiness)"),
         opt("source", "luma", "height from luma, r, g, b or a"),
         opt("edge", "repeat", "border handling: repeat (tiling), clamp, mirror"),
         opt("flip-x", "0", "invert the red channel"),
         opt("flip-y", "0", "invert green (default is OpenGL style +Y up; flip-y=1 for DirectX style)"),
         opt("height-alpha", "0", "store the height in alpha (for parallax mapping)")},
        createNormalMap);
}

} // namespace img
} // namespace idt3
