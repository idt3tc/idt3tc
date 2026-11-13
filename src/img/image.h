#pragma once

#include <cstddef>
#include <string>
#include <vector>

namespace idt3 {
namespace img {

// An RGBA color with float channels, nominally in [0, 1].
struct Color {
    float r, g, b, a;
    Color() : r(0), g(0), b(0), a(0) {}
    Color(float r_, float g_, float b_, float a_) : r(r_), g(g_), b(b_), a(a_) {}
    float operator[](int i) const { return i == 0 ? r : i == 1 ? g : i == 2 ? b : a; }
};

// Working image: 4 float channels (RGBA), rows stored top to bottom.
//
// All operations work in this representation so that chains of operations
// do not accumulate 8-bit rounding errors. Color values are kept in the
// encoding they were loaded in (normally sRGB); operations that care about
// light-linear math offer an explicit `linear` option.
struct Image {
    int width;
    int height;
    std::vector<float> data;

    Image() : width(0), height(0) {}
    Image(int w, int h) : width(0), height(0) { allocate(w, h); }
    Image(int w, int h, const Color& fill) : width(0), height(0) { allocate(w, h, fill); }

    // Resizes the pixel buffer; contents are zeroed or set to `fill`.
    void allocate(int w, int h, const Color& fill = Color());

    bool empty() const { return width <= 0 || height <= 0; }
    size_t pixelCount() const { return static_cast<size_t>(width) * static_cast<size_t>(height); }

    float* pixel(int x, int y) { return &data[(static_cast<size_t>(y) * width + x) * 4]; }
    const float* pixel(int x, int y) const { return &data[(static_cast<size_t>(y) * width + x) * 4]; }
    float* row(int y) { return pixel(0, y); }
    const float* row(int y) const { return pixel(0, y); }
};

// Maximum supported dimension; guards against absurd allocations from
// typos like "--resize 100000x100000".
const int kMaxDimension = 65536;

// Throws if w/h are outside [1, kMaxDimension].
void checkDimensions(int w, int h, const char* what);

// Float <-> 8 bit conversion with rounding and clamping.
inline unsigned char toByte(float v)
{
    if (!(v > 0.0f))
        return 0;
    if (v >= 1.0f)
        return 255;
    return static_cast<unsigned char>(v * 255.0f + 0.5f);
}

inline float clamp01(float v)
{
    return v < 0.0f ? 0.0f : (v > 1.0f ? 1.0f : v);
}

float srgbToLinear(float v);
float linearToSrgb(float v);

// In-place conversions applied to the RGB channels.
void toLinear(Image& img);
void toSrgb(Image& img);
void premultiply(Image& img);
// Inverse of premultiply; fully transparent pixels get black RGB.
void unpremultiply(Image& img);
void clampImage(Image& img);

// Quantized RGBA8 buffer (row-major, top-down).
std::vector<unsigned char> toRGBA8(const Image& img);
Image fromRGBA8(const unsigned char* rgba, int w, int h);

// Alpha characteristics after 8-bit quantization, used to pick output
// formats automatically.
struct AlphaInfo {
    bool opaque;  // every alpha is 255
    bool binary;  // every alpha is 0 or 255
    unsigned char min;
    unsigned char max;
};
AlphaInfo analyzeAlpha(const Image& img);

// True if R == G == B for every pixel after 8-bit quantization.
bool isGrayscale(const Image& img);

inline bool isPowerOfTwo(int v)
{
    return v > 0 && (v & (v - 1)) == 0;
}

// Rec. 709 luma of an RGB triple.
inline float luma(const float* p)
{
    return 0.2126f * p[0] + 0.7152f * p[1] + 0.0722f * p[2];
}

} // namespace img
} // namespace idt3
