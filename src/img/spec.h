#pragma once

#include "img/image.h"
#include "img/sampling.h"

#include <string>
#include <vector>

// Parsers for the value syntaxes used on the command line.
namespace idt3 {
namespace img {

// A length that is either absolute (pixels) or relative to a reference
// length ("50%").
struct Length {
    double value;
    bool percent;
    Length() : value(0), percent(false) {}
    Length(double v, bool pct) : value(v), percent(pct) {}
    double resolve(double reference) const { return percent ? value * reference / 100.0 : value; }
};

bool parseLength(const std::string& s, Length& out);

// "[W][xH][{+-}X{+-}Y]" with W, H, X, Y either pixels or percentages,
// e.g. "512x512", "512x", "x256", "50%", "256x256+16+0", "+32-8".
struct Geometry {
    bool hasW, hasH, hasOffset;
    Length w, h, x, y;
    Geometry() : hasW(false), hasH(false), hasOffset(false) {}
};

// Throws UsageError with a descriptive message on malformed input.
Geometry parseGeometry(const std::string& s, const std::string& what);

// "X,Y" pair of lengths (pixels or percent), e.g. "32,-16" or "50%,0".
void parseLengthPair(const std::string& s, const std::string& what, Length& x, Length& y);

// Colors: "#rgb", "#rgba", "#rrggbb", "#rrggbbaa" (the last two also
// without '#'), "r,g,b[,a]" (0-255),
// or a name (black, white, transparent, gray, red, green, blue, ...).
bool parseColor(const std::string& s, Color& out);

bool parseEdgeMode(const std::string& s, EdgeMode& out);
std::string edgeModeName(EdgeMode m);

bool parseInterp(const std::string& s, Interp& out);

// Placement of a region relative to a container.
enum class Gravity { NorthWest, North, NorthEast, West, Center, East, SouthWest, South, SouthEast };
bool parseGravity(const std::string& s, Gravity& out);

// Origin of a `w x h` region inside a `cw x ch` container for a gravity,
// before applying an offset.
void gravityOrigin(Gravity g, int cw, int ch, int w, int h, int& x, int& y);

// Channel set such as "rgb", "a", "rgba" -> bitmask (r=1, g=2, b=4, a=8).
bool parseChannels(const std::string& s, int& mask);

} // namespace img
} // namespace idt3
