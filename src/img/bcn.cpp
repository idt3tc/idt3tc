#include "img/bcn.h"

#include "common/parallel.h"
#include "common/util.h"

#include <cmp_core.h>

#include <algorithm>
#include <cstring>

namespace idt3 {
namespace img {

namespace {

// RAII holder for CMP_Core option contexts.
struct CmpOptions {
    void* ptr;
    int (*destroy)(void*);
    CmpOptions() : ptr(nullptr), destroy(nullptr) {}
    ~CmpOptions()
    {
        if (ptr && destroy)
            destroy(ptr);
    }
};

void check(int rc, const char* what)
{
    if (rc != 0)
        throw Error(strprintf("Compressonator: %s failed (error %d)", what, rc));
}

void setupOptions(CmpOptions& o, BlockFormat f, const BlockOptions& opts)
{
    const float q = std::max(0.0f, std::min(1.0f, opts.quality));
    switch (f) {
    case BlockFormat::BC1:
    case BlockFormat::BC1A:
        check(CreateOptionsBC1(&o.ptr), "CreateOptionsBC1");
        o.destroy = DestroyOptionsBC1;
        check(SetQualityBC1(o.ptr, q), "SetQualityBC1");
        check(SetRefineStepsBC1(o.ptr, static_cast<unsigned>(opts.refineSteps)), "SetRefineStepsBC1");
        if (opts.useWeights)
            check(SetChannelWeightsBC1(o.ptr, opts.weights[0], opts.weights[1], opts.weights[2]),
                  "SetChannelWeightsBC1");
        break;
    case BlockFormat::BC2:
        check(CreateOptionsBC2(&o.ptr), "CreateOptionsBC2");
        o.destroy = DestroyOptionsBC2;
        check(SetQualityBC2(o.ptr, q), "SetQualityBC2");
        if (opts.useWeights)
            check(SetChannelWeightsBC2(o.ptr, opts.weights[0], opts.weights[1], opts.weights[2]),
                  "SetChannelWeightsBC2");
        break;
    case BlockFormat::BC3:
        check(CreateOptionsBC3(&o.ptr), "CreateOptionsBC3");
        o.destroy = DestroyOptionsBC3;
        check(SetQualityBC3(o.ptr, q), "SetQualityBC3");
        if (opts.useWeights)
            check(SetChannelWeightsBC3(o.ptr, opts.weights[0], opts.weights[1], opts.weights[2]),
                  "SetChannelWeightsBC3");
        break;
    case BlockFormat::BC4:
        check(CreateOptionsBC4(&o.ptr), "CreateOptionsBC4");
        o.destroy = DestroyOptionsBC4;
        check(SetQualityBC4(o.ptr, q), "SetQualityBC4");
        break;
    case BlockFormat::BC5:
        check(CreateOptionsBC5(&o.ptr), "CreateOptionsBC5");
        o.destroy = DestroyOptionsBC5;
        check(SetQualityBC5(o.ptr, q), "SetQualityBC5");
        break;
    case BlockFormat::BC7:
        check(CreateOptionsBC7(&o.ptr), "CreateOptionsBC7");
        o.destroy = DestroyOptionsBC7;
        check(SetQualityBC7(o.ptr, q), "SetQualityBC7");
        break;
    }
}

// --- BC1 helpers ------------------------------------------------------------
//
// CMP_Core's BC1 encoder ignores the alpha threshold, and may pick the
// 3-color mode (color0 <= color1) whose 4th index decodes as transparent
// black on most hardware. For idTech 3 this matters: opaque DXT1 must
// never contain such texels, and DXT1a needs real punch-through alpha. Both
// are handled here by re-indexing against CMP_Core's endpoints.

void decode565(unsigned c, int rgb[3])
{
    int r = (c >> 11) & 31, g = (c >> 5) & 63, b = c & 31;
    rgb[0] = (r << 3) | (r >> 2);
    rgb[1] = (g << 2) | (g >> 4);
    rgb[2] = (b << 3) | (b >> 2);
}

int colorDistance(const int a[3], const unsigned char* b, const float w[3])
{
    float d = 0;
    for (int c = 0; c < 3; ++c) {
        float diff = static_cast<float>(a[c] - b[c]);
        d += diff * diff * w[c];
    }
    return static_cast<int>(d * 16.0f);
}

void writeBc1(unsigned char out[8], unsigned c0, unsigned c1, unsigned indices)
{
    out[0] = static_cast<unsigned char>(c0 & 0xff);
    out[1] = static_cast<unsigned char>(c0 >> 8);
    out[2] = static_cast<unsigned char>(c1 & 0xff);
    out[3] = static_cast<unsigned char>(c1 >> 8);
    out[4] = static_cast<unsigned char>(indices & 0xff);
    out[5] = static_cast<unsigned char>((indices >> 8) & 0xff);
    out[6] = static_cast<unsigned char>((indices >> 16) & 0xff);
    out[7] = static_cast<unsigned char>(indices >> 24);
}

// Re-indexes `block` (16 RGBA texels) against endpoints c0/c1.
// fourColor: c0 > c1 is required by the caller; otherwise 3-color mode with
// index 3 used for texels flagged in `transparent`.
unsigned reindex(const unsigned char* block, unsigned c0, unsigned c1, bool fourColor, const bool* transparent,
                 const float w[3])
{
    int p[4][3];
    decode565(c0, p[0]);
    decode565(c1, p[1]);
    int numColors = fourColor ? 4 : 3;
    for (int c = 0; c < 3; ++c) {
        if (fourColor) {
            p[2][c] = (2 * p[0][c] + p[1][c]) / 3;
            p[3][c] = (p[0][c] + 2 * p[1][c]) / 3;
        } else {
            p[2][c] = (p[0][c] + p[1][c]) / 2;
        }
    }
    unsigned indices = 0;
    for (int i = 0; i < 16; ++i) {
        unsigned idx;
        if (!fourColor && transparent && transparent[i]) {
            idx = 3;
        } else {
            int best = 0, bestDist = colorDistance(p[0], block + i * 4, w);
            for (int k = 1; k < numColors; ++k) {
                int d = colorDistance(p[k], block + i * 4, w);
                if (d < bestDist) {
                    bestDist = d;
                    best = k;
                }
            }
            idx = static_cast<unsigned>(best);
        }
        indices |= idx << (2 * i);
    }
    return indices;
}

void compressBc1(const unsigned char* block, unsigned char out[8], bool punchThrough, const BlockOptions& opts,
                 const void* cmpOptions)
{
    const float unitWeights[3] = {1, 1, 1};
    const float* w = opts.useWeights ? opts.weights : unitWeights;

    bool transparent[16];
    int numTransparent = 0;
    for (int i = 0; i < 16; ++i) {
        transparent[i] = punchThrough && block[i * 4 + 3] < opts.alphaThreshold;
        numTransparent += transparent[i] ? 1 : 0;
    }

    if (numTransparent == 16) {
        // Fully transparent: 3-color mode, every index 3.
        writeBc1(out, 0, 0, 0xffffffffu);
        return;
    }

    // Transparent texels must not influence the endpoints: replace them by
    // the average opaque color before running the encoder.
    unsigned char work[64];
    std::memcpy(work, block, 64);
    if (numTransparent > 0) {
        int sum[3] = {0, 0, 0};
        for (int i = 0; i < 16; ++i) {
            if (!transparent[i]) {
                for (int c = 0; c < 3; ++c)
                    sum[c] += block[i * 4 + c];
            }
        }
        int opaqueCount = 16 - numTransparent;
        for (int i = 0; i < 16; ++i) {
            if (transparent[i]) {
                for (int c = 0; c < 3; ++c)
                    work[i * 4 + c] = static_cast<unsigned char>(sum[c] / opaqueCount);
            }
        }
    }
    for (int i = 0; i < 16; ++i)
        work[i * 4 + 3] = 255;

    unsigned char cmp[8];
    check(CompressBlockBC1(work, 16, cmp, cmpOptions), "CompressBlockBC1");
    unsigned c0 = cmp[0] | (cmp[1] << 8);
    unsigned c1 = cmp[2] | (cmp[3] << 8);

    if (numTransparent > 0) {
        // 3-color mode requires c0 <= c1.
        unsigned lo = std::min(c0, c1), hi = std::max(c0, c1);
        writeBc1(out, lo, hi, reindex(work, lo, hi, false, transparent, w));
        return;
    }

    if (c0 > c1) {
        std::memcpy(out, cmp, 8); // already 4-color mode, keep encoder result
        return;
    }
    // Convert to 4-color mode: order endpoints descending; equal endpoints
    // get nudged apart by one LSB of blue so that c0 > c1 holds.
    unsigned hi = std::max(c0, c1), lo = std::min(c0, c1);
    if (hi == lo) {
        if (lo > 0)
            --lo;
        else
            ++hi;
    }
    writeBc1(out, hi, lo, reindex(work, hi, lo, true, nullptr, w));
}

// Fills a 4x4 RGBA block from the surface, replicating edge texels.
void gatherBlock(const unsigned char* rgba, int w, int h, int bx, int by, unsigned char block[64])
{
    for (int y = 0; y < 4; ++y) {
        int sy = std::min(by * 4 + y, h - 1);
        for (int x = 0; x < 4; ++x) {
            int sx = std::min(bx * 4 + x, w - 1);
            std::memcpy(block + (y * 4 + x) * 4, rgba + (static_cast<size_t>(sy) * w + sx) * 4, 4);
        }
    }
}

} // namespace

size_t blockBytes(BlockFormat f)
{
    return (f == BlockFormat::BC1 || f == BlockFormat::BC1A || f == BlockFormat::BC4) ? 8 : 16;
}

size_t compressedSize(BlockFormat f, int w, int h)
{
    size_t bw = static_cast<size_t>((w + 3) / 4), bh = static_cast<size_t>((h + 3) / 4);
    return bw * bh * blockBytes(f);
}

std::vector<unsigned char> compressSurface(const unsigned char* rgba, int w, int h, BlockFormat f,
                                           const BlockOptions& opts, int jobs)
{
    const int bw = (w + 3) / 4, bh = (h + 3) / 4;
    const size_t bsize = blockBytes(f);
    std::vector<unsigned char> out(compressedSize(f, w, h));

    CmpOptions cmpOpts;
    setupOptions(cmpOpts, f, opts);
    const void* o = cmpOpts.ptr;

    // Block rows are independent; CMP_Core keeps no mutable global state in
    // the encode paths, and the option contexts are only read.
    parallelFor(bh, jobs, [&](int y0, int y1) {
        unsigned char block[64];
        unsigned char chan[2][16];
        for (int by = y0; by < y1; ++by) {
            for (int bx = 0; bx < bw; ++bx) {
                gatherBlock(rgba, w, h, bx, by, block);
                unsigned char* dst = &out[(static_cast<size_t>(by) * bw + bx) * bsize];
                switch (f) {
                case BlockFormat::BC1:
                    compressBc1(block, dst, false, opts, o);
                    break;
                case BlockFormat::BC1A:
                    compressBc1(block, dst, true, opts, o);
                    break;
                case BlockFormat::BC2:
                    check(CompressBlockBC2(block, 16, dst, o), "CompressBlockBC2");
                    break;
                case BlockFormat::BC3:
                    check(CompressBlockBC3(block, 16, dst, o), "CompressBlockBC3");
                    break;
                case BlockFormat::BC4:
                    for (int i = 0; i < 16; ++i)
                        chan[0][i] = block[i * 4];
                    check(CompressBlockBC4(chan[0], 4, dst, o), "CompressBlockBC4");
                    break;
                case BlockFormat::BC5:
                    for (int i = 0; i < 16; ++i) {
                        chan[0][i] = block[i * 4];
                        chan[1][i] = block[i * 4 + 1];
                    }
                    check(CompressBlockBC5(chan[0], 4, chan[1], 4, dst, o), "CompressBlockBC5");
                    break;
                case BlockFormat::BC7:
                    check(CompressBlockBC7(block, 16, dst, o), "CompressBlockBC7");
                    break;
                }
            }
        }
    });
    return out;
}

namespace {

// BC1 decoding done locally so 3-color mode is decoded as on hardware
// (index 3 = transparent black), independent of CMP_Core's decoder flavor.
void decodeBc1Block(const unsigned char* src, unsigned char out[64], bool forceFourColor)
{
    unsigned c0 = src[0] | (src[1] << 8);
    unsigned c1 = src[2] | (src[3] << 8);
    int p[4][4];
    decode565(c0, p[0]);
    decode565(c1, p[1]);
    p[0][3] = p[1][3] = 255;
    bool four = forceFourColor || c0 > c1;
    for (int c = 0; c < 3; ++c) {
        if (four) {
            p[2][c] = (2 * p[0][c] + p[1][c]) / 3;
            p[3][c] = (p[0][c] + 2 * p[1][c]) / 3;
        } else {
            p[2][c] = (p[0][c] + p[1][c]) / 2;
            p[3][c] = 0;
        }
    }
    p[2][3] = 255;
    p[3][3] = four ? 255 : 0;
    unsigned idx = src[4] | (src[5] << 8) | (src[6] << 16) | (static_cast<unsigned>(src[7]) << 24);
    for (int i = 0; i < 16; ++i) {
        int k = (idx >> (2 * i)) & 3;
        for (int c = 0; c < 4; ++c)
            out[i * 4 + c] = static_cast<unsigned char>(p[k][c]);
    }
}

} // namespace

std::vector<unsigned char> decompressSurface(const unsigned char* data, size_t size, int w, int h, BlockFormat f)
{
    if (size < compressedSize(f, w, h))
        throw Error("compressed surface is truncated");
    const int bw = (w + 3) / 4, bh = (h + 3) / 4;
    const size_t bsize = blockBytes(f);
    std::vector<unsigned char> out(static_cast<size_t>(w) * h * 4);
    unsigned char block[64];
    unsigned char chan[2][16];
    for (int by = 0; by < bh; ++by) {
        for (int bx = 0; bx < bw; ++bx) {
            const unsigned char* src = data + (static_cast<size_t>(by) * bw + bx) * bsize;
            switch (f) {
            case BlockFormat::BC1:
            case BlockFormat::BC1A:
                decodeBc1Block(src, block, false);
                break;
            case BlockFormat::BC2:
            case BlockFormat::BC3: {
                // Color part of BC2/BC3 is always 4-color mode.
                decodeBc1Block(src + 8, block, true);
                unsigned char tmp[64];
                if (f == BlockFormat::BC2)
                    check(DecompressBlockBC2(src, tmp), "DecompressBlockBC2");
                else
                    check(DecompressBlockBC3(src, tmp), "DecompressBlockBC3");
                for (int i = 0; i < 16; ++i)
                    block[i * 4 + 3] = tmp[i * 4 + 3];
                break;
            }
            case BlockFormat::BC4:
                check(DecompressBlockBC4(src, chan[0]), "DecompressBlockBC4");
                for (int i = 0; i < 16; ++i) {
                    block[i * 4] = block[i * 4 + 1] = block[i * 4 + 2] = chan[0][i];
                    block[i * 4 + 3] = 255;
                }
                break;
            case BlockFormat::BC5:
                check(DecompressBlockBC5(src, chan[0], chan[1]), "DecompressBlockBC5");
                for (int i = 0; i < 16; ++i) {
                    block[i * 4] = chan[0][i];
                    block[i * 4 + 1] = chan[1][i];
                    block[i * 4 + 2] = 0;
                    block[i * 4 + 3] = 255;
                }
                break;
            case BlockFormat::BC7:
                check(DecompressBlockBC7(src, block), "DecompressBlockBC7");
                break;
            }
            for (int y = 0; y < 4; ++y) {
                int py = by * 4 + y;
                if (py >= h)
                    break;
                for (int x = 0; x < 4; ++x) {
                    int px = bx * 4 + x;
                    if (px >= w)
                        break;
                    std::memcpy(&out[(static_cast<size_t>(py) * w + px) * 4], block + (y * 4 + x) * 4, 4);
                }
            }
        }
    }
    return out;
}

} // namespace img
} // namespace idt3
