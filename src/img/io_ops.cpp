// Operations that create, read or write images: new, load, save, info,
// channel-from, composite.

#include "common/fs.h"
#include "common/util.h"
#include "img/io.h"
#include "img/op_util.h"
#include "img/resample.h"

#include <algorithm>
#include <cstdio>

namespace idt3 {
namespace img {

namespace {

const char* const kGravityHelp = "anchor: nw, n, ne, w, center, e, sw, s, se";

std::string sizeString(size_t bytes)
{
    if (bytes < 10 * 1024)
        return strprintf("%zu B", bytes);
    if (bytes < 10 * 1024 * 1024)
        return strprintf("%.1f KiB", bytes / 1024.0);
    return strprintf("%.1f MiB", bytes / (1024.0 * 1024.0));
}

// --- new / load -----------------------------------------------------------------

std::unique_ptr<Op> createNew(const std::string& arg, OptionReader& o)
{
    Geometry g = parseGeometry(arg, "--new");
    if (!g.hasW || !g.hasH || g.w.percent || g.h.percent || g.hasOffset)
        throw UsageError("--new: expected absolute WxH, e.g. 256x256");
    const int w = roundToInt(g.w.value), h = roundToInt(g.h.value);
    checkDimensions(w, h, "--new");
    Color c = o.color("color", Color(0, 0, 0, 1));
    return makeOp([=](Context& ctx) {
        ctx.image = Image(w, h, c);
        ctx.hasImage = true;
    });
}

std::unique_ptr<Op> createLoad(const std::string& arg, OptionReader&)
{
    return makeOp([arg](Context& ctx) {
        std::string path = ctx.expandPath(arg);
        LoadInfo li;
        ctx.image = loadImage(path, &li);
        ctx.hasImage = true;
        ctx.verbose(strprintf("loaded %s (%dx%d, %s)", path.c_str(), ctx.image.width, ctx.image.height,
                              li.format.c_str()));
    });
}

// --- save -----------------------------------------------------------------------

enum class Format { Tga, Dds, Png, Jpg, Bmp };

bool formatFromName(const std::string& n, Format& f)
{
    std::string s = toLower(n);
    if (s == "tga")
        f = Format::Tga;
    else if (s == "dds")
        f = Format::Dds;
    else if (s == "png")
        f = Format::Png;
    else if (s == "jpg" || s == "jpeg")
        f = Format::Jpg;
    else if (s == "bmp")
        f = Format::Bmp;
    else
        return false;
    return true;
}

// Parses "full", "none" or a total level count (1 = base level only).
int parseMipLevels(OptionReader& o)
{
    std::string s = toLower(o.str("mips", "full"));
    if (s == "full" || s == "all")
        return 0;
    if (s == "none" || s == "0")
        return 1;
    long n;
    if (!parseLong(s, n) || n < 1 || n > 17)
        throw UsageError("mips: expected full, none or a level count 1..17, got '" + s + "'");
    return static_cast<int>(n);
}

std::unique_ptr<Op> createSave(const std::string& arg, OptionReader& o)
{
    if (arg.empty())
        throw UsageError("--save: empty output path");
    Format fmt;
    std::string fmtName = o.str("format", "auto");
    if (toLower(fmtName) == "auto") {
        if (!formatFromName(fs::extension(arg), fmt))
            throw UsageError("--save: cannot derive the format from '" + arg +
                             "'; use a .tga/.dds/.png/.jpg/.bmp extension or format=...");
    } else if (!formatFromName(fmtName, fmt)) {
        throw UsageError("--save: unknown format '" + fmtName + "' (expected tga, dds, png, jpg or bmp)");
    }
    const bool mkdir = o.flag("mkdir", true);

    // Only the options that apply to the chosen format are read; finish()
    // then reports e.g. codec= on a TGA output as an error.
    TgaOptions tga;
    DdsOptions dds;
    int channels = 0, level = 9, quality = 95;
    switch (fmt) {
    case Format::Tga: {
        int bpp = o.choice("bpp", {"auto", "8", "24", "32"}, 0);
        tga.bpp = bpp == 0 ? 0 : bpp == 1 ? 8 : bpp == 2 ? 24 : 32;
        tga.rle = o.flag("rle", false);
        tga.topOrigin = o.choice("origin", {"bottom", "top"}, 0) == 1;
        break;
    }
    case Format::Dds: {
        std::string c = o.str("codec", "auto");
        if (!parseDdsCodec(c, dds.codec))
            throw UsageError("--save: unknown DDS codec '" + c + "' (expected " + join(ddsCodecNames(), ", ") +
                             ")");
        dds.mips.levels = parseMipLevels(o);
        dds.mips.minSize = o.integer("mip-min", 1, 1, kMaxDimension);
        dds.mips.resample = o.resampleOptions(Filter::Box, EdgeMode::Repeat, "mip-");
        dds.mips.coverageRef = static_cast<float>(o.number("mip-coverage", 0.0, 0.0, 1.0));
        // BC7 gains under 1 dB PSNR from 0.1 to 1.0 on photographic content
        // at roughly 8x the encoding time; BC1-5 are fast at any setting.
        const double defQuality = dds.codec == DdsCodec::BC7 ? 0.1 : 1.0;
        dds.block.quality = static_cast<float>(o.number("quality", defQuality, 0.0, 1.0));
        dds.block.refineSteps = o.integer("refine", 0, 0, 16);
        if (o.has("weights")) {
            std::string w = toLower(o.str("weights", ""));
            dds.block.useWeights = true;
            if (w != "perceptual") {
                std::vector<std::string> p = split(w, ',');
                double v[3];
                if (p.size() != 3 || !parseDouble(p[0], v[0]) || !parseDouble(p[1], v[1]) ||
                    !parseDouble(p[2], v[2]) || v[0] < 0 || v[1] < 0 || v[2] < 0 || v[0] + v[1] + v[2] <= 0)
                    throw UsageError("weights: expected 'perceptual' or R,G,B (non-negative numbers)");
                // Normalized so the error metric scale stays comparable.
                double sum = v[0] + v[1] + v[2];
                for (int i = 0; i < 3; ++i)
                    dds.block.weights[i] = static_cast<float>(v[i] / sum);
            }
        }
        dds.block.alphaThreshold = o.integer("alpha-threshold", 128, 1, 255);
        int dx10 = o.choice("dx10", {"auto", "0", "1"}, 0);
        dds.dx10 = dx10 == 0 ? -1 : dx10 == 1 ? 0 : 1;
        dds.srgb = o.flag("srgb", false);
        break;
    }
    case Format::Png:
        level = o.integer("compression", 9, 0, 9);
        channels = o.choice("channels", {"auto", "1", "2", "3", "4"}, 0);
        break;
    case Format::Jpg:
        quality = o.integer("quality", 95, 1, 100);
        break;
    case Format::Bmp:
        channels = o.choice("channels", {"auto", "3", "4"}, 0);
        channels = channels == 0 ? 0 : channels + 2;
        break;
    }

    return makeOp(
        [=](Context& ctx) {
            const Image& img = ctx.requireImage("--save");
            const std::string path = ctx.expandPath(arg);
            if (mkdir) {
                std::string dir = fs::dirname(path);
                if (!dir.empty())
                    fs::createDirectories(dir);
            }

            std::vector<unsigned char> bytes;
            std::string desc;
            switch (fmt) {
            case Format::Tga: {
                int bpp = 0;
                bytes = encodeTga(img, tga, &bpp);
                desc = strprintf("tga %d-bit%s", bpp, tga.rle ? " rle" : "");
                if (tga.rle && bpp == 8)
                    ctx.info("warning: " + path +
                             ": RLE grayscale TGAs (type 11) are not supported by the idTech 3 loader");
                break;
            }
            case Format::Dds: {
                DdsResult r = encodeDds(img, dds, ctx.jobs);
                bytes.swap(r.bytes);
                desc = strprintf("dds %s, %d mip%s%s", ddsCodecName(r.codec).c_str(), r.mipLevels,
                                 r.mipLevels == 1 ? "" : "s", r.dx10 ? ", dx10" : "");
                break;
            }
            case Format::Png:
                bytes = encodePng(img, channels, level);
                desc = "png";
                break;
            case Format::Jpg:
                bytes = encodeJpg(img, quality);
                desc = strprintf("jpg q%d", quality);
                break;
            case Format::Bmp:
                bytes = encodeBmp(img, channels);
                desc = "bmp";
                break;
            }
            fs::writeFile(path, bytes);
            ctx.info(strprintf("wrote %s (%dx%d, %s, %s)", path.c_str(), img.width, img.height, desc.c_str(),
                               sizeString(bytes.size()).c_str()));
        },
        arg);
}

// --- info -----------------------------------------------------------------------

// Named function instead of a captureless lambda: GCC 16 reports a false
// -Wmaybe-uninitialized for empty closures stored in std::function.
void applyInfo(Context& ctx)
{
    const Image& img = ctx.requireImage("--info");
    AlphaInfo a = analyzeAlpha(img);
    std::string name = ctx.inputPath.empty() ? std::string("<image>") : ctx.inputPath;
    std::string alpha = a.opaque ? "opaque" : a.binary ? "binary (1-bit)" : "gradient";
    std::printf("%s: %dx%d%s, alpha: %s (min %d, max %d)%s\n", name.c_str(), img.width, img.height,
                isPowerOfTwo(img.width) && isPowerOfTwo(img.height) ? " (power of two)" : " (not power of two)",
                alpha.c_str(), static_cast<int>(a.min), static_cast<int>(a.max), isGrayscale(img) ? ", grayscale" : "");
    std::fflush(stdout);
}

std::unique_ptr<Op> createInfo(const std::string&, OptionReader&)
{
    return makeOp(applyInfo);
}

// --- channel-from ---------------------------------------------------------------

std::unique_ptr<Op> createChannelFrom(const std::string& arg, OptionReader& o)
{
    const int from = o.choice("from", {"r", "g", "b", "a", "luma"}, 4);
    const int to = o.channels("to", 8);
    const bool resize = o.flag("resize", true);
    ResampleOptions rs = o.resampleOptions(Filter::Mitchell, EdgeMode::Clamp);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--channel-from");
        std::string path = ctx.expandPath(arg);
        Image src = loadImage(path);
        if (src.width != img.width || src.height != img.height) {
            if (!resize)
                throw Error(strprintf("--channel-from: %s is %dx%d, expected %dx%d (or use resize=1)", path.c_str(),
                                      src.width, src.height, img.width, img.height));
            src = resample(src, img.width, img.height, rs, ctx.jobs);
        }
        for (size_t i = 0; i < img.data.size(); i += 4) {
            const float* s = &src.data[i];
            float v = from == 4 ? luma(s) : s[from];
            for (int c = 0; c < 4; ++c) {
                if (to & (1 << c))
                    img.data[i + c] = clamp01(v);
            }
        }
    });
}

// --- composite ------------------------------------------------------------------

enum class Blend { Over, Add, Multiply, Screen, Replace };

std::unique_ptr<Op> createComposite(const std::string& arg, OptionReader& o)
{
    const Gravity gravity = o.gravity("gravity", Gravity::NorthWest);
    const int ox = o.integer("x", 0, -kMaxDimension, kMaxDimension);
    const int oy = o.integer("y", 0, -kMaxDimension, kMaxDimension);
    const Blend blend = static_cast<Blend>(o.choice("blend", {"over", "add", "multiply", "screen", "replace"}, 0));
    const float opacity = static_cast<float>(o.number("opacity", 1.0, 0.0, 1.0));
    const bool tile = o.flag("tile", false);
    return makeOp([=](Context& ctx) {
        Image& dst = ctx.requireImage("--composite");
        const std::string path = ctx.expandPath(arg);
        const Image src = loadImage(path);
        int bx = 0, by = 0;
        if (!tile)
            gravityOrigin(gravity, dst.width, dst.height, src.width, src.height, bx, by);
        bx += ox;
        by += oy;
        for (int y = 0; y < dst.height; ++y) {
            int sy = y - by;
            if (tile)
                sy = ((sy % src.height) + src.height) % src.height;
            else if (sy < 0 || sy >= src.height)
                continue;
            for (int x = 0; x < dst.width; ++x) {
                int sx = x - bx;
                if (tile)
                    sx = ((sx % src.width) + src.width) % src.width;
                else if (sx < 0 || sx >= src.width)
                    continue;
                const float* s = src.pixel(sx, sy);
                float* d = dst.pixel(x, y);
                const float sa = s[3] * opacity;
                float res[4];
                switch (blend) {
                case Blend::Over: {
                    // Porter-Duff source-over on straight alpha.
                    float oa = sa + d[3] * (1.0f - sa);
                    for (int c = 0; c < 3; ++c)
                        res[c] = oa > 0 ? (s[c] * sa + d[c] * d[3] * (1.0f - sa)) / oa : 0.0f;
                    res[3] = oa;
                    break;
                }
                case Blend::Replace:
                    for (int c = 0; c < 4; ++c)
                        res[c] = d[c] + (s[c] - d[c]) * opacity;
                    break;
                default:
                    for (int c = 0; c < 3; ++c) {
                        float b = blend == Blend::Add        ? d[c] + s[c]
                                  : blend == Blend::Multiply ? d[c] * s[c]
                                                             : 1.0f - (1.0f - d[c]) * (1.0f - s[c]);
                        res[c] = d[c] + (clamp01(b) - d[c]) * sa;
                    }
                    res[3] = d[3];
                    break;
                }
                for (int c = 0; c < 4; ++c)
                    d[c] = clamp01(res[c]);
            }
        }
    });
}

} // namespace

void registerIoOps(OpRegistry& r)
{
    const std::string cat = "Input & output";
    auto add = [&r](const std::string& name, const std::vector<std::string>& aliases, const std::string& argName,
                    const std::string& category, const std::string& summary, const std::string& details,
                    const std::vector<OptionHelp>& options, OpFactory create) {
        OpInfo i;
        i.name = name;
        i.aliases = aliases;
        i.arg = argName;
        i.category = category;
        i.summary = summary;
        i.details = details;
        i.options = options;
        i.create = create;
        r.add(i);
    };

    add("new", {}, "WxH", cat, "start from a blank image", "", {opt("color", "black", "fill color")}, createNew);
    add("load", {}, "FILE", cat, "replace the current image with FILE (path placeholders are expanded)", "", {},
        createLoad);
    add("save", {"o", "output"}, "PATH", cat, "write the current image; the pipeline continues afterwards",
        "The format follows the extension (.tga, .dds, .png, .jpg, .bmp) unless format= is given. PATH may "
        "contain placeholders: {name} input file stem, {ext} input extension, {dir} input directory, {rel} "
        "path below a directory input (without extension), {index}, {w}, {h}; {{ and }} for literal braces. "
        "Only the options of the chosen format are accepted.",
        withOptions(
            {opt("format", "auto", "tga, dds, png, jpg, bmp"),
             opt("mkdir", "1", "create missing parent directories"),
             opt("bpp", "auto", "TGA: auto (32 with alpha, else 24), 8 (grayscale), 24, 32"),
             opt("rle", "0", "TGA: run-length compression (supported by idTech 3 for 24/32 bit)"),
             opt("origin", "bottom", "TGA: row order; ioquake3 ignores the flag, keep bottom for idTech 3"),
             opt("codec", "auto", "DDS: " + join(ddsCodecNames(), ", ") +
                                      "; auto = dxt1 when opaque, else dxt5"),
             opt("mips", "full", "DDS: full, none or total level count"),
             opt("mip-min", "1", "DDS: smallest mip dimension to generate")},
            withOptions(resampleOptionHelp("box", "repeat", "mip-"),
                        {opt("mip-coverage", "0",
                             "DDS: keep alpha-test coverage in mips at this alpha reference (e.g. 0.5); 0 = off"),
                         opt("quality", "", "DDS: block compression quality 0..1, lower is faster (default 1, bc7: 0.1); "
                                             "JPG: 1..100 (default 95)"),
                         opt("refine", "0", "DDS: extra BC1 endpoint refinement steps"),
                         opt("weights", "", "DDS: color error weights 'perceptual' or R,G,B (BC1-3)"),
                         opt("alpha-threshold", "128", "DDS bc1a: alpha below this becomes transparent"),
                         opt("dx10", "auto", "DDS: write the DX10 header: auto (only when needed), 0, 1"),
                         opt("srgb", "0", "DDS: tag the format as sRGB (requires the DX10 header)"),
                         opt("compression", "9", "PNG: zlib level 0..9"),
                         opt("channels", "auto", "PNG: auto, 1 (gray), 2 (gray+alpha), 3, 4; BMP: auto, 3, 4")})),
        createSave);
    add("info", {}, "", cat, "print size and alpha information to stdout", "", {}, createInfo);
    add("channel-from", {}, "FILE", cat, "copy one channel of FILE into channels of the current image",
        "Typical use: --channel-from mask.png from=luma to=a to attach a separate alpha mask.",
        withOptions({opt("from", "luma", "source channel: r, g, b, a or luma"),
                     opt("to", "a", "destination channels, any of r, g, b, a"),
                     opt("resize", "1", "resample FILE when its size differs")},
                    resampleOptionHelp("mitchell", "clamp")),
        createChannelFrom);
    add("composite", {"overlay"}, "FILE", cat, "draw FILE on top of the current image", "",
        {opt("gravity", "nw", kGravityHelp), opt("x", "0", "horizontal offset in pixels"),
         opt("y", "0", "vertical offset in pixels"),
         opt("blend", "over", "over, add, multiply, screen, replace"),
         opt("opacity", "1", "0..1"), opt("tile", "0", "repeat FILE over the whole image")},
        createComposite);
}

} // namespace img
} // namespace idt3
