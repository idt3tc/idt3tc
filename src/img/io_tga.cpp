#include "img/io.h"

#include "common/util.h"

#include <cstring>

namespace idt3 {
namespace img {

namespace {

void put16(std::vector<unsigned char>& out, int v)
{
    out.push_back(static_cast<unsigned char>(v & 0xff));
    out.push_back(static_cast<unsigned char>((v >> 8) & 0xff));
}

// Appends one scanline as RLE packets. Packets never cross scanlines,
// as recommended by the TGA 2.0 spec (some loaders cannot handle it).
void rleScanline(const unsigned char* row, int width, int bpp, std::vector<unsigned char>& out)
{
    const int px = bpp / 8;
    int i = 0;
    while (i < width) {
        // Length of the run of identical pixels starting at i.
        int run = 1;
        while (i + run < width && run < 128 && std::memcmp(row + i * px, row + (i + run) * px, px) == 0)
            ++run;
        if (run >= 2) {
            out.push_back(static_cast<unsigned char>(0x80 | (run - 1)));
            out.insert(out.end(), row + i * px, row + (i + 1) * px);
            i += run;
            continue;
        }
        // Raw packet: extend until a repeat starts or 128 pixels.
        int start = i;
        int count = 0;
        while (i < width && count < 128) {
            if (i + 1 < width && std::memcmp(row + i * px, row + (i + 1) * px, px) == 0)
                break;
            ++i;
            ++count;
        }
        out.push_back(static_cast<unsigned char>(count - 1));
        out.insert(out.end(), row + start * px, row + (start + count) * px);
    }
}

} // namespace

std::vector<unsigned char> encodeTga(const Image& img, const TgaOptions& opts, int* bppUsed)
{
    if (img.empty())
        throw Error("TGA: empty image");
    if (img.width > 65535 || img.height > 65535)
        throw Error("TGA: image too large (max 65535x65535)");

    int bpp = opts.bpp;
    if (bpp == 0)
        bpp = analyzeAlpha(img).opaque ? 24 : 32;
    if (bpp != 8 && bpp != 24 && bpp != 32)
        throw UsageError(strprintf("TGA: unsupported bit depth %d (use 8, 24 or 32)", bpp));
    if (bppUsed)
        *bppUsed = bpp;

    const bool gray = bpp == 8;
    std::vector<unsigned char> out;
    out.reserve(18 + img.pixelCount() * (bpp / 8));
    out.push_back(0); // no image ID
    out.push_back(0); // no color map
    // 2 = truecolor, 3 = grayscale, +8 = RLE
    out.push_back(static_cast<unsigned char>((gray ? 3 : 2) + (opts.rle ? 8 : 0)));
    for (int i = 0; i < 5; ++i)
        out.push_back(0); // color map spec
    put16(out, 0);        // x origin
    put16(out, 0);        // y origin
    put16(out, img.width);
    put16(out, img.height);
    out.push_back(static_cast<unsigned char>(bpp));
    // Descriptor: alpha bits in the low nibble, bit 5 = top-down rows.
    out.push_back(static_cast<unsigned char>((bpp == 32 ? 8 : 0) | (opts.topOrigin ? 0x20 : 0)));

    const int px = bpp / 8;
    std::vector<unsigned char> row(static_cast<size_t>(img.width) * px);
    for (int i = 0; i < img.height; ++i) {
        const int y = opts.topOrigin ? i : img.height - 1 - i;
        const float* src = img.row(y);
        for (int x = 0; x < img.width; ++x) {
            const float* p = src + x * 4;
            unsigned char* d = &row[static_cast<size_t>(x) * px];
            if (gray) {
                d[0] = toByte(luma(p));
            } else {
                // TGA stores BGR(A).
                d[0] = toByte(p[2]);
                d[1] = toByte(p[1]);
                d[2] = toByte(p[0]);
                if (px == 4)
                    d[3] = toByte(p[3]);
            }
        }
        if (opts.rle)
            rleScanline(&row[0], img.width, bpp, out);
        else
            out.insert(out.end(), row.begin(), row.end());
    }
    return out;
}

} // namespace img
} // namespace idt3
