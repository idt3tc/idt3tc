// Geometric operations: resize, pot, crop, canvas, pad, trim, rotate,
// flip, transpose, pan, tile.

#include "common/util.h"
#include "img/op_util.h"
#include "img/resample.h"
#include "img/transform.h"

#include <algorithm>
#include <cmath>

namespace idt3 {
namespace img {

namespace {

const char* const kGravityHelp = "anchor: nw, n, ne, w, center, e, sw, s, se";

// --- resize -------------------------------------------------------------------

enum class Fit { Stretch, Contain, Cover };
enum class Only { Always, Shrink, Enlarge };

std::unique_ptr<Op> createResize(const std::string& arg, OptionReader& o)
{
    Geometry g = parseGeometry(arg, "--resize");
    if (g.hasOffset)
        throw UsageError("--resize: offsets are not allowed (use --crop or --canvas)");
    Fit fit = static_cast<Fit>(o.choice("fit", {"stretch", "contain", "cover"}, 0));
    Only only = static_cast<Only>(o.choice("only", {"always", "shrink", "enlarge"}, 0));
    Gravity gravity = o.gravity("gravity", Gravity::Center);
    ResampleOptions rs = o.resampleOptions(Filter::Mitchell, EdgeMode::Clamp);

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--resize");
        const double sw = img.width, sh = img.height;
        double tw, th;
        if (g.hasW && g.hasH) {
            tw = g.w.resolve(sw);
            th = g.h.resolve(sh);
        } else if (g.hasW) {
            tw = g.w.resolve(sw);
            th = sh * tw / sw;
        } else {
            th = g.h.resolve(sh);
            tw = sw * th / sh;
        }

        int rw = std::max(1, roundToInt(tw)), rh = std::max(1, roundToInt(th));
        int cropW = 0, cropH = 0;
        if (g.hasW && g.hasH && fit != Fit::Stretch) {
            double s = fit == Fit::Contain ? std::min(tw / sw, th / sh) : std::max(tw / sw, th / sh);
            if (fit == Fit::Cover) {
                cropW = rw;
                cropH = rh;
            }
            rw = std::max(1, roundToInt(sw * s));
            rh = std::max(1, roundToInt(sh * s));
        }

        const int finalW = cropW ? cropW : rw, finalH = cropH ? cropH : rh;
        if (only == Only::Shrink && finalW >= img.width && finalH >= img.height) {
            ctx.verbose("--resize: skipped (only=shrink)");
            return;
        }
        if (only == Only::Enlarge && finalW <= img.width && finalH <= img.height) {
            ctx.verbose("--resize: skipped (only=enlarge)");
            return;
        }

        Image out = resample(img, rw, rh, rs, ctx.jobs);
        if (cropW) {
            int x, y;
            gravityOrigin(gravity, rw, rh, cropW, cropH, x, y);
            out = extractRegion(out, x, y, cropW, cropH, EdgeMode::Clamp, Color());
        }
        ctx.verbose(strprintf("--resize: %dx%d -> %dx%d", img.width, img.height, out.width, out.height));
        img = out;
    });
}

// --- pot ----------------------------------------------------------------------

enum class PotMode { Nearest, Up, Down };

int roundPot(int n, PotMode mode)
{
    int down = 1;
    while (down * 2 <= n)
        down *= 2;
    int up = down == n ? n : down * 2;
    if (mode == PotMode::Down)
        return down;
    if (mode == PotMode::Up)
        return up;
    return (n - down) < (up - n) ? down : up; // ties round up
}

std::unique_ptr<Op> createPot(const std::string&, OptionReader& o)
{
    PotMode mode = static_cast<PotMode>(o.choice("mode", {"nearest", "up", "down"}, 0));
    int maxSize = o.integer("max", 0, 0, kMaxDimension);
    int minSize = o.integer("min", 1, 1, kMaxDimension);
    if (maxSize && !isPowerOfTwo(maxSize))
        throw UsageError("--pot: max must be a power of two");
    if (!isPowerOfTwo(minSize))
        throw UsageError("--pot: min must be a power of two");
    bool square = o.flag("square", false);
    bool canvas = o.choice("method", {"resample", "canvas"}, 0) == 1;
    Gravity gravity = o.gravity("gravity", Gravity::Center);
    ResampleOptions rs = o.resampleOptions(Filter::Mitchell, canvas ? EdgeMode::Fill : EdgeMode::Clamp);

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--pot");
        int w = roundPot(img.width, mode), h = roundPot(img.height, mode);
        if (square)
            w = h = (mode == PotMode::Down) ? std::min(w, h) : std::max(w, h);
        if (maxSize) {
            w = std::min(w, maxSize);
            h = std::min(h, maxSize);
        }
        w = std::max(w, minSize);
        h = std::max(h, minSize);
        if (w == img.width && h == img.height)
            return;
        ctx.verbose(strprintf("--pot: %dx%d -> %dx%d", img.width, img.height, w, h));
        if (canvas) {
            int x, y;
            gravityOrigin(gravity, img.width, img.height, w, h, x, y);
            img = extractRegion(img, x, y, w, h, rs.edge, rs.fill);
        } else {
            img = resample(img, w, h, rs, ctx.jobs);
        }
    });
}

// --- crop / canvas ----------------------------------------------------------

// Resolves geometry to a region inside (or around) the current image.
void resolveRegion(const Geometry& g, Gravity gravity, const Image& img, int& x, int& y, int& w, int& h)
{
    w = g.hasW ? roundToInt(g.w.resolve(img.width)) : img.width;
    h = g.hasH ? roundToInt(g.h.resolve(img.height)) : img.height;
    if (g.hasW && !g.hasH && !g.w.percent)
        h = img.height; // "256" alone: width only, full height
    if (w < 1 || h < 1)
        throw Error("region size resolves to less than one pixel");
    gravityOrigin(gravity, img.width, img.height, w, h, x, y);
    if (g.hasOffset) {
        x += roundToInt(g.x.resolve(img.width));
        y += roundToInt(g.y.resolve(img.height));
    }
}

std::unique_ptr<Op> createCrop(const std::string& arg, OptionReader& o)
{
    Geometry g = parseGeometry(arg, "--crop");
    Gravity gravity = o.gravity("gravity", Gravity::NorthWest);
    bool clip = o.flag("clip", true);
    EdgeMode edge = o.edge("edge", EdgeMode::Fill);
    Color bg = o.color("bg", Color());

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--crop");
        int x, y, w, h;
        resolveRegion(g, gravity, img, x, y, w, h);
        if (clip) {
            int x0 = std::max(0, x), y0 = std::max(0, y);
            int x1 = std::min(img.width, x + w), y1 = std::min(img.height, y + h);
            if (x1 <= x0 || y1 <= y0)
                throw Error(strprintf("--crop: region %dx%d%+d%+d lies outside the %dx%d image", w, h, x, y,
                                      img.width, img.height));
            x = x0;
            y = y0;
            w = x1 - x0;
            h = y1 - y0;
        }
        ctx.verbose(strprintf("--crop: %dx%d%+d%+d", w, h, x, y));
        img = extractRegion(img, x, y, w, h, edge, bg);
    });
}

std::unique_ptr<Op> createCanvas(const std::string& arg, OptionReader& o)
{
    Geometry g = parseGeometry(arg, "--canvas");
    Gravity gravity = o.gravity("gravity", Gravity::Center);
    EdgeMode edge = o.edge("edge", EdgeMode::Fill);
    Color bg = o.color("bg", Color());

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--canvas");
        int x, y, w, h;
        resolveRegion(g, gravity, img, x, y, w, h);
        checkDimensions(w, h, "--canvas");
        img = extractRegion(img, x, y, w, h, edge, bg);
    });
}

// --- pad ----------------------------------------------------------------------

std::unique_ptr<Op> createPad(const std::string& arg, OptionReader& o)
{
    std::vector<std::string> parts = split(arg, ',');
    long v[4];
    for (size_t i = 0; i < parts.size() && i < 4; ++i) {
        if (!parseLong(parts[i], v[i]) || v[i] < -kMaxDimension || v[i] > kMaxDimension)
            throw UsageError("--pad: invalid value '" + parts[i] + "'");
    }
    int left, top, right, bottom;
    if (parts.size() == 1) {
        left = top = right = bottom = static_cast<int>(v[0]);
    } else if (parts.size() == 2) {
        left = right = static_cast<int>(v[0]);
        top = bottom = static_cast<int>(v[1]);
    } else if (parts.size() == 4) {
        left = static_cast<int>(v[0]);
        top = static_cast<int>(v[1]);
        right = static_cast<int>(v[2]);
        bottom = static_cast<int>(v[3]);
    } else {
        throw UsageError("--pad: expected N, H,V or L,T,R,B");
    }
    EdgeMode edge = o.edge("edge", EdgeMode::Fill);
    Color bg = o.color("bg", Color());

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--pad");
        int w = img.width + left + right, h = img.height + top + bottom;
        if (w < 1 || h < 1)
            throw Error("--pad: negative padding removes the whole image");
        img = extractRegion(img, -left, -top, w, h, edge, bg);
    });
}

// --- trim ---------------------------------------------------------------------

std::unique_ptr<Op> createTrim(const std::string&, OptionReader& o)
{
    int mode = o.choice("mode", {"auto", "alpha", "color"}, 0);
    bool hasColor = o.has("color");
    Color color = o.color("color", Color());
    float fuzz = static_cast<float>(o.number("fuzz", 0.0, 0.0, 1.0));
    int margin = o.integer("margin", 0, 0, kMaxDimension);
    if (hasColor && mode == 1)
        throw UsageError("--trim: color= cannot be combined with mode=alpha");

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--trim");
        // Decide what counts as "border": transparent pixels or pixels
        // matching a color (explicit or taken from the top-left corner).
        bool byAlpha = mode == 1;
        Color ref = color;
        if (mode == 0 && !hasColor) {
            const float* corner = img.pixel(0, 0);
            byAlpha = corner[3] <= fuzz;
            ref = Color(corner[0], corner[1], corner[2], corner[3]);
        }
        auto isBorder = [&](int x, int y) {
            const float* p = img.pixel(x, y);
            if (byAlpha)
                return p[3] <= fuzz;
            for (int c = 0; c < 4; ++c) {
                if (std::fabs(p[c] - ref[c]) > fuzz + 1e-6f)
                    return false;
            }
            return true;
        };
        int x0 = img.width, y0 = img.height, x1 = -1, y1 = -1;
        for (int y = 0; y < img.height; ++y) {
            for (int x = 0; x < img.width; ++x) {
                if (!isBorder(x, y)) {
                    x0 = std::min(x0, x);
                    y0 = std::min(y0, y);
                    x1 = std::max(x1, x);
                    y1 = std::max(y1, y);
                }
            }
        }
        if (x1 < 0) {
            ctx.info("--trim: image consists only of border pixels, left unchanged");
            return;
        }
        x0 = std::max(0, x0 - margin);
        y0 = std::max(0, y0 - margin);
        x1 = std::min(img.width - 1, x1 + margin);
        y1 = std::min(img.height - 1, y1 + margin);
        ctx.verbose(strprintf("--trim: %dx%d+%d+%d", x1 - x0 + 1, y1 - y0 + 1, x0, y0));
        img = extractRegion(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1, EdgeMode::Clamp, Color());
    });
}

// --- rotate -------------------------------------------------------------------

AffineOptions affineOptions(OptionReader& o, Interp defInterp, EdgeMode defEdge)
{
    AffineOptions a;
    a.interp = o.interp("filter", defInterp);
    a.edge = o.edge("edge", defEdge);
    a.fill = o.color("bg", Color());
    a.premultiply = o.flag("premultiply", true);
    return a;
}

std::vector<OptionHelp> affineHelp(const std::string& defInterp, const std::string& defEdge)
{
    std::vector<OptionHelp> h;
    h.push_back(opt("filter", defInterp, "interpolation: nearest, bilinear, bicubic"));
    h.push_back(opt("edge", defEdge, "source outside the image: fill, clamp, repeat, mirror"));
    h.push_back(opt("bg", "transparent", "color used with edge=fill"));
    h.push_back(opt("premultiply", "1", "interpolate alpha-weighted colors"));
    return h;
}

std::unique_ptr<Op> createRotate(const std::string& arg, OptionReader& o)
{
    double deg;
    if (!parseDouble(arg, deg))
        throw UsageError("--rotate: invalid angle '" + arg + "'");
    bool expand = o.flag("expand", true);
    AffineOptions a = affineOptions(o, Interp::Bicubic, EdgeMode::Fill);

    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--rotate");
        double norm = std::fmod(deg, 360.0);
        if (norm < 0)
            norm += 360.0;
        double q = norm / 90.0;
        int quarters = roundToInt(q);
        bool exactQuarter = std::fabs(q - quarters) < 1e-9;
        // Quarter turns are lossless unless a non-square image must keep
        // its canvas (expand=0 with 90/270 degrees).
        if (exactQuarter && (expand || quarters % 2 == 0 || img.width == img.height)) {
            img = rotateQuarter(img, quarters);
            return;
        }
        img = rotateArbitrary(img, norm, expand, a, ctx.jobs);
    });
}

// --- flip / transpose -------------------------------------------------------

std::unique_ptr<Op> createFlip(const std::string& arg, OptionReader&)
{
    std::string a = toLower(arg);
    bool h = false, v = false;
    if (a == "h" || a == "x" || a == "horizontal")
        h = true;
    else if (a == "v" || a == "y" || a == "vertical")
        v = true;
    else if (a == "hv" || a == "vh" || a == "xy" || a == "both")
        h = v = true;
    else
        throw UsageError("--flip: expected h, v or hv, got '" + arg + "'");
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--flip");
        if (h)
            img = flipHorizontal(img);
        if (v)
            img = flipVertical(img);
    });
}

// Named function instead of a captureless lambda: GCC 16 reports a false
// -Wmaybe-uninitialized for empty closures stored in std::function.
void applyTranspose(Context& ctx)
{
    Image& img = ctx.requireImage("--transpose");
    img = transpose(img);
}

std::unique_ptr<Op> createTranspose(const std::string&, OptionReader&)
{
    return makeOp(applyTranspose);
}

// --- pan ----------------------------------------------------------------------

std::unique_ptr<Op> createPan(const std::string& arg, OptionReader& o)
{
    Length lx, ly;
    parseLengthPair(arg, "--pan", lx, ly);
    AffineOptions a = affineOptions(o, Interp::Bilinear, EdgeMode::Repeat);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--pan");
        img = translate(img, lx.resolve(img.width), ly.resolve(img.height), a, ctx.jobs);
    });
}

// --- tile ---------------------------------------------------------------------

std::unique_ptr<Op> createTile(const std::string& arg, OptionReader& o)
{
    std::string a = toLower(arg);
    size_t x = a.find('x');
    long nx, ny;
    if (x == std::string::npos) {
        if (!parseLong(a, nx))
            throw UsageError("--tile: expected N or NxM, got '" + arg + "'");
        ny = nx;
    } else if (!parseLong(a.substr(0, x), nx) || !parseLong(a.substr(x + 1), ny)) {
        throw UsageError("--tile: expected N or NxM, got '" + arg + "'");
    }
    if (nx < 1 || ny < 1 || nx > 256 || ny > 256)
        throw UsageError("--tile: counts must be in 1..256");
    bool mirror = o.flag("mirror", false);
    return makeOp([=](Context& ctx) {
        Image& img = ctx.requireImage("--tile");
        long w = img.width * nx, h = img.height * ny;
        checkDimensions(static_cast<int>(std::min<long>(w, kMaxDimension + 1)),
                        static_cast<int>(std::min<long>(h, kMaxDimension + 1)), "--tile");
        img = extractRegion(img, 0, 0, static_cast<int>(w), static_cast<int>(h),
                            mirror ? EdgeMode::Mirror : EdgeMode::Repeat, Color());
    });
}

} // namespace

void registerGeometryOps(OpRegistry& r)
{
    const std::string cat = "Geometry";
    {
        OpInfo i;
        i.name = "resize";
        i.aliases = {"scale"};
        i.arg = "GEOMETRY";
        i.category = cat;
        i.summary = "resample to a new size";
        i.details = "GEOMETRY is WxH, Wx or W (keep aspect, compute height), xH (compute width), N% or "
                    "W%xH%. With both W and H given, fit= decides how the aspect ratio is handled.";
        i.options = withOptions(
            {opt("fit", "stretch",
                 "stretch: exactly WxH; contain: largest size fitting into WxH; cover: fill WxH, crop the overflow"),
             opt("only", "always", "always, shrink (never enlarge) or enlarge (never shrink)"),
             opt("gravity", "center", std::string("crop anchor for fit=cover; ") + kGravityHelp)},
            resampleOptionHelp("mitchell", "clamp"));
        i.create = createResize;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "pot";
        i.category = cat;
        i.summary = "make both dimensions powers of two (required by idTech 3 for undistorted textures)";
        i.details = "Rounds each dimension to a power of two, then resamples (method=resample) or pads/crops "
                    "the canvas (method=canvas).";
        i.options = withOptions(
            {opt("mode", "nearest", "rounding: nearest (ties go up), up, down"),
             opt("max", "0", "upper limit per dimension (power of two), 0 = none; e.g. 2048 to stay within "
                             "the texture size limits of older engines/drivers"),
             opt("min", "1", "lower limit per dimension (power of two)"),
             opt("square", "0", "force a square result"),
             opt("method", "resample", "resample: scale the image; canvas: pad/crop without scaling"),
             opt("gravity", "center", std::string("method=canvas anchor; ") + kGravityHelp)},
            resampleOptionHelp("mitchell", "clamp (fill for method=canvas)"));
        i.create = createPot;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "crop";
        i.arg = "GEOMETRY";
        i.category = cat;
        i.summary = "cut out a region";
        i.details = "GEOMETRY is WxH+X+Y (also WxH, +X+Y, percentages). The offset is relative to the gravity "
                    "anchor, e.g. '256x256 gravity=center' takes the middle.";
        i.options = {opt("gravity", "nw", kGravityHelp),
                     opt("clip", "1", "clip the region to the image; with clip=0 outside parts use edge/bg"),
                     opt("edge", "fill", "with clip=0: fill, clamp, repeat, mirror"),
                     opt("bg", "transparent", "fill color for edge=fill")};
        i.create = createCrop;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "canvas";
        i.aliases = {"extent"};
        i.arg = "GEOMETRY";
        i.category = cat;
        i.summary = "change the canvas size without scaling (pad or crop)";
        i.details = "GEOMETRY is WxH[+X+Y]; the image is placed by gravity, new areas are filled per edge=.";
        i.options = {opt("gravity", "center", kGravityHelp),
                     opt("edge", "fill", "new areas: fill (bg color), clamp, repeat, mirror"),
                     opt("bg", "transparent", "fill color")};
        i.create = createCanvas;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "pad";
        i.aliases = {"border"};
        i.arg = "N|H,V|L,T,R,B";
        i.category = cat;
        i.summary = "add (or with negative values remove) pixels at the borders";
        i.options = {opt("edge", "fill", "new areas: fill (bg color), clamp, repeat, mirror"),
                     opt("bg", "transparent", "fill color")};
        i.create = createPad;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "trim";
        i.category = cat;
        i.summary = "remove uniform or transparent borders";
        i.options = {opt("mode", "auto", "auto (alpha if the top-left pixel is transparent, else its color), "
                                         "alpha, color"),
                     opt("color", "", "border color (implies mode=color semantics)"),
                     opt("fuzz", "0", "tolerance per channel (0..1)"),
                     opt("margin", "0", "keep this many pixels around the content")};
        i.create = createTrim;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "rotate";
        i.arg = "DEGREES";
        i.category = cat;
        i.summary = "rotate clockwise (multiples of 90 are lossless)";
        i.options = withOptions({opt("expand", "1", "grow the canvas to fit the rotated image")},
                                affineHelp("bicubic", "fill"));
        i.create = createRotate;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "flip";
        i.aliases = {"mirror"};
        i.arg = "h|v|hv";
        i.category = cat;
        i.summary = "mirror horizontally (h), vertically (v) or both";
        i.create = createFlip;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "transpose";
        i.category = cat;
        i.summary = "swap rows and columns (mirror along the main diagonal)";
        i.create = createTranspose;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "pan";
        i.aliases = {"offset", "shift", "roll"};
        i.arg = "DX,DY";
        i.category = cat;
        i.summary = "shift the content; wraps around by default (seam checking)";
        i.details = "DX,DY in pixels (fractional allowed) or percent, e.g. 50%,50% moves the texture seams to "
                    "the center. Positive values move right/down.";
        i.options = affineHelp("bilinear", "repeat");
        i.create = createPan;
        r.add(i);
    }
    {
        OpInfo i;
        i.name = "tile";
        i.arg = "N|NxM";
        i.category = cat;
        i.summary = "repeat the image N x M times";
        i.options = {opt("mirror", "0", "mirror every other copy")};
        i.create = createTile;
        r.add(i);
    }
}

} // namespace img
} // namespace idt3
