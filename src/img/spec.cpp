#include "img/spec.h"

#include "common/util.h"

#include <cctype>
#include <cstdlib>

namespace idt3 {
namespace img {

bool parseLength(const std::string& s, Length& out)
{
    if (s.empty())
        return false;
    bool pct = s[s.size() - 1] == '%';
    double v;
    if (!parseDouble(pct ? s.substr(0, s.size() - 1) : s, v))
        return false;
    out = Length(v, pct);
    return true;
}

Geometry parseGeometry(const std::string& s, const std::string& what)
{
    Geometry g;
    std::string size = s;
    std::string offset;

    // The offset starts at the first '+' or '-' that follows the size part
    // (a leading sign would be meaningless for a size).
    size_t p = s.find_first_of("+-");
    if (p != std::string::npos) {
        size = s.substr(0, p);
        offset = s.substr(p);
    }

    if (!size.empty()) {
        size_t xpos = toLower(size).find('x');
        std::string ws = xpos == std::string::npos ? size : size.substr(0, xpos);
        std::string hs = xpos == std::string::npos ? std::string() : size.substr(xpos + 1);
        if (!ws.empty()) {
            if (!parseLength(ws, g.w) || g.w.value <= 0)
                throw UsageError(what + ": invalid width '" + ws + "' in '" + s + "'");
            g.hasW = true;
        }
        if (!hs.empty()) {
            if (!parseLength(hs, g.h) || g.h.value <= 0)
                throw UsageError(what + ": invalid height '" + hs + "' in '" + s + "'");
            g.hasH = true;
        }
        // A bare "50%" scales both axes, like ImageMagick.
        if (xpos == std::string::npos && g.w.percent) {
            g.h = g.w;
            g.hasH = true;
        }
    }

    if (!offset.empty()) {
        // Expect exactly two signed components: {+-}X{+-}Y.
        size_t second = offset.find_first_of("+-", 1);
        if (second == std::string::npos)
            throw UsageError(what + ": offset needs both X and Y (e.g. +16+0) in '" + s + "'");
        std::string xs = offset.substr(0, second);
        std::string ys = offset.substr(second);
        if (xs[0] == '+')
            xs.erase(0, 1);
        if (ys[0] == '+')
            ys.erase(0, 1);
        if (!parseLength(xs, g.x) || !parseLength(ys, g.y))
            throw UsageError(what + ": invalid offset '" + offset + "' in '" + s + "'");
        g.hasOffset = true;
    }

    if (!g.hasW && !g.hasH && !g.hasOffset)
        throw UsageError(what + ": empty geometry");
    return g;
}

void parseLengthPair(const std::string& s, const std::string& what, Length& x, Length& y)
{
    std::vector<std::string> parts = split(s, ',');
    if (parts.size() != 2 || !parseLength(parts[0], x) || !parseLength(parts[1], y))
        throw UsageError(what + ": expected 'X,Y' (pixels or percent), got '" + s + "'");
}

namespace {

int hexDigit(char c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    return -1;
}

struct NamedColor {
    const char* name;
    unsigned char r, g, b, a;
};

const NamedColor kNamedColors[] = {
    {"black", 0, 0, 0, 255},       {"white", 255, 255, 255, 255}, {"transparent", 0, 0, 0, 0},
    {"none", 0, 0, 0, 0},          {"gray", 128, 128, 128, 255},  {"grey", 128, 128, 128, 255},
    {"red", 255, 0, 0, 255},       {"green", 0, 255, 0, 255},     {"blue", 0, 0, 255, 255},
    {"yellow", 255, 255, 0, 255},  {"cyan", 0, 255, 255, 255},    {"magenta", 255, 0, 255, 255},
    // Flat tangent-space normal (0, 0, 1).
    {"normal", 128, 128, 255, 255},
};

} // namespace

bool parseColor(const std::string& sIn, Color& out)
{
    std::string s = toLower(sIn);
    if (s.empty())
        return false;
    for (size_t i = 0; i < sizeof(kNamedColors) / sizeof(kNamedColors[0]); ++i) {
        if (s == kNamedColors[i].name) {
            const NamedColor& n = kNamedColors[i];
            out = Color(n.r / 255.0f, n.g / 255.0f, n.b / 255.0f, n.a / 255.0f);
            return true;
        }
    }
    // '#' starts a comment in shells when it begins a word, so 6/8 digit
    // hex is also accepted without it ("ff8000").
    bool bareHex = (s.size() == 6 || s.size() == 8) && s.find_first_not_of("0123456789abcdef") == std::string::npos;
    if (s[0] == '#' || bareHex) {
        std::string h = bareHex ? s : s.substr(1);
        std::vector<int> d;
        for (size_t i = 0; i < h.size(); ++i) {
            int v = hexDigit(h[i]);
            if (v < 0)
                return false;
            d.push_back(v);
        }
        int comps[4] = {0, 0, 0, 255};
        if (h.size() == 3 || h.size() == 4) {
            for (size_t i = 0; i < h.size(); ++i)
                comps[i] = d[i] * 17;
        } else if (h.size() == 6 || h.size() == 8) {
            for (size_t i = 0; i < h.size() / 2; ++i)
                comps[i] = d[2 * i] * 16 + d[2 * i + 1];
        } else {
            return false;
        }
        out = Color(comps[0] / 255.0f, comps[1] / 255.0f, comps[2] / 255.0f, comps[3] / 255.0f);
        return true;
    }
    std::vector<std::string> parts = split(s, ',');
    if (parts.size() == 3 || parts.size() == 4) {
        double v[4] = {0, 0, 0, 255};
        for (size_t i = 0; i < parts.size(); ++i) {
            if (!parseDouble(parts[i], v[i]) || v[i] < 0 || v[i] > 255)
                return false;
        }
        out = Color(static_cast<float>(v[0] / 255.0), static_cast<float>(v[1] / 255.0),
                    static_cast<float>(v[2] / 255.0), static_cast<float>(v[3] / 255.0));
        return true;
    }
    return false;
}

bool parseEdgeMode(const std::string& sIn, EdgeMode& out)
{
    std::string s = toLower(sIn);
    if (s == "clamp" || s == "edge")
        out = EdgeMode::Clamp;
    else if (s == "repeat" || s == "wrap" || s == "tile")
        out = EdgeMode::Repeat;
    else if (s == "mirror" || s == "reflect")
        out = EdgeMode::Mirror;
    else if (s == "fill" || s == "background" || s == "bg")
        out = EdgeMode::Fill;
    else
        return false;
    return true;
}

std::string edgeModeName(EdgeMode m)
{
    switch (m) {
    case EdgeMode::Clamp:
        return "clamp";
    case EdgeMode::Repeat:
        return "repeat";
    case EdgeMode::Mirror:
        return "mirror";
    case EdgeMode::Fill:
        return "fill";
    }
    return "?";
}

bool parseInterp(const std::string& sIn, Interp& out)
{
    std::string s = toLower(sIn);
    if (s == "nearest" || s == "point")
        out = Interp::Nearest;
    else if (s == "bilinear" || s == "linear" || s == "triangle")
        out = Interp::Bilinear;
    else if (s == "bicubic" || s == "cubic" || s == "catmullrom")
        out = Interp::Bicubic;
    else
        return false;
    return true;
}

bool parseGravity(const std::string& sIn, Gravity& out)
{
    std::string s = toLower(sIn);
    if (s == "nw" || s == "northwest" || s == "topleft" || s == "top-left")
        out = Gravity::NorthWest;
    else if (s == "n" || s == "north" || s == "top")
        out = Gravity::North;
    else if (s == "ne" || s == "northeast" || s == "topright" || s == "top-right")
        out = Gravity::NorthEast;
    else if (s == "w" || s == "west" || s == "left")
        out = Gravity::West;
    else if (s == "c" || s == "center" || s == "centre")
        out = Gravity::Center;
    else if (s == "e" || s == "east" || s == "right")
        out = Gravity::East;
    else if (s == "sw" || s == "southwest" || s == "bottomleft" || s == "bottom-left")
        out = Gravity::SouthWest;
    else if (s == "s" || s == "south" || s == "bottom")
        out = Gravity::South;
    else if (s == "se" || s == "southeast" || s == "bottomright" || s == "bottom-right")
        out = Gravity::SouthEast;
    else
        return false;
    return true;
}

void gravityOrigin(Gravity g, int cw, int ch, int w, int h, int& x, int& y)
{
    int col, row; // 0 = left/top, 1 = center, 2 = right/bottom
    switch (g) {
    case Gravity::NorthWest: col = 0; row = 0; break;
    case Gravity::North:     col = 1; row = 0; break;
    case Gravity::NorthEast: col = 2; row = 0; break;
    case Gravity::West:      col = 0; row = 1; break;
    case Gravity::Center:    col = 1; row = 1; break;
    case Gravity::East:      col = 2; row = 1; break;
    case Gravity::SouthWest: col = 0; row = 2; break;
    case Gravity::South:     col = 1; row = 2; break;
    default:                 col = 2; row = 2; break;
    }
    // Floor division keeps odd differences consistent for negative values.
    auto place = [](int mode, int container, int size) {
        int diff = container - size;
        if (mode == 0)
            return 0;
        if (mode == 2)
            return diff;
        return diff >= 0 ? diff / 2 : -((-diff + 1) / 2);
    };
    x = place(col, cw, w);
    y = place(row, ch, h);
}

bool parseChannels(const std::string& sIn, int& mask)
{
    std::string s = toLower(sIn);
    if (s.empty())
        return false;
    if (s == "all") {
        mask = 15;
        return true;
    }
    mask = 0;
    for (size_t i = 0; i < s.size(); ++i) {
        switch (s[i]) {
        case 'r': mask |= 1; break;
        case 'g': mask |= 2; break;
        case 'b': mask |= 4; break;
        case 'a': mask |= 8; break;
        default: return false;
        }
    }
    return true;
}

} // namespace img
} // namespace idt3
