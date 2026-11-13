#include "img/image.h"

#include "common/util.h"

#include <cmath>

namespace idt3 {
namespace img {

void Image::allocate(int w, int h, const Color& fill)
{
    width = w;
    height = h;
    data.assign(static_cast<size_t>(w) * static_cast<size_t>(h) * 4, 0.0f);
    if (fill.r != 0 || fill.g != 0 || fill.b != 0 || fill.a != 0) {
        for (size_t i = 0; i < data.size(); i += 4) {
            data[i + 0] = fill.r;
            data[i + 1] = fill.g;
            data[i + 2] = fill.b;
            data[i + 3] = fill.a;
        }
    }
}

void checkDimensions(int w, int h, const char* what)
{
    if (w < 1 || h < 1 || w > kMaxDimension || h > kMaxDimension)
        throw Error(strprintf("%s: invalid image size %dx%d (allowed: 1..%d)", what, w, h, kMaxDimension));
}

float srgbToLinear(float v)
{
    if (v <= 0.04045f)
        return v / 12.92f;
    return std::pow((v + 0.055f) / 1.055f, 2.4f);
}

float linearToSrgb(float v)
{
    if (v <= 0.0031308f)
        return v * 12.92f;
    return 1.055f * std::pow(v, 1.0f / 2.4f) - 0.055f;
}

void toLinear(Image& img)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        for (int c = 0; c < 3; ++c)
            img.data[i + c] = srgbToLinear(img.data[i + c]);
    }
}

void toSrgb(Image& img)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        for (int c = 0; c < 3; ++c)
            img.data[i + c] = linearToSrgb(img.data[i + c] < 0 ? 0 : img.data[i + c]);
    }
}

void premultiply(Image& img)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        float a = img.data[i + 3];
        img.data[i + 0] *= a;
        img.data[i + 1] *= a;
        img.data[i + 2] *= a;
    }
}

void unpremultiply(Image& img)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        float a = img.data[i + 3];
        if (a > 1e-6f) {
            img.data[i + 0] /= a;
            img.data[i + 1] /= a;
            img.data[i + 2] /= a;
        } else {
            img.data[i + 0] = img.data[i + 1] = img.data[i + 2] = 0.0f;
        }
    }
}

void clampImage(Image& img)
{
    for (size_t i = 0; i < img.data.size(); ++i)
        img.data[i] = clamp01(img.data[i]);
}

std::vector<unsigned char> toRGBA8(const Image& img)
{
    std::vector<unsigned char> out(img.data.size());
    for (size_t i = 0; i < img.data.size(); ++i)
        out[i] = toByte(img.data[i]);
    return out;
}

Image fromRGBA8(const unsigned char* rgba, int w, int h)
{
    Image img(w, h);
    for (size_t i = 0; i < img.data.size(); ++i)
        img.data[i] = rgba[i] / 255.0f;
    return img;
}

AlphaInfo analyzeAlpha(const Image& img)
{
    AlphaInfo info;
    info.opaque = true;
    info.binary = true;
    info.min = 255;
    info.max = 0;
    for (size_t i = 3; i < img.data.size(); i += 4) {
        unsigned char a = toByte(img.data[i]);
        if (a != 255)
            info.opaque = false;
        if (a != 0 && a != 255)
            info.binary = false;
        if (a < info.min)
            info.min = a;
        if (a > info.max)
            info.max = a;
    }
    if (img.data.empty())
        info.min = info.max = 255;
    return info;
}

bool isGrayscale(const Image& img)
{
    for (size_t i = 0; i < img.data.size(); i += 4) {
        unsigned char r = toByte(img.data[i]);
        if (toByte(img.data[i + 1]) != r || toByte(img.data[i + 2]) != r)
            return false;
    }
    return true;
}

} // namespace img
} // namespace idt3
