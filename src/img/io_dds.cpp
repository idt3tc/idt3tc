#include "img/io.h"

#include "common/util.h"

#include <algorithm>
#include <cstring>

namespace idt3 {
namespace img {

namespace {

// DDS_PIXELFORMAT flags
const unsigned DDPF_ALPHAPIXELS = 0x1;
const unsigned DDPF_ALPHA = 0x2;
const unsigned DDPF_FOURCC = 0x4;
const unsigned DDPF_RGB = 0x40;
const unsigned DDPF_LUMINANCE = 0x20000;

// DDS_HEADER flags
const unsigned DDSD_CAPS = 0x1;
const unsigned DDSD_HEIGHT = 0x2;
const unsigned DDSD_WIDTH = 0x4;
const unsigned DDSD_PITCH = 0x8;
const unsigned DDSD_PIXELFORMAT = 0x1000;
const unsigned DDSD_MIPMAPCOUNT = 0x20000;
const unsigned DDSD_LINEARSIZE = 0x80000;

const unsigned DDSCAPS_COMPLEX = 0x8;
const unsigned DDSCAPS_TEXTURE = 0x1000;
const unsigned DDSCAPS_MIPMAP = 0x400000;

const unsigned DDS_RESOURCE_DIMENSION_TEXTURE2D = 3;

unsigned fourCC(const char* s)
{
    return static_cast<unsigned>(static_cast<unsigned char>(s[0])) |
           static_cast<unsigned>(static_cast<unsigned char>(s[1])) << 8 |
           static_cast<unsigned>(static_cast<unsigned char>(s[2])) << 16 |
           static_cast<unsigned>(static_cast<unsigned char>(s[3])) << 24;
}

// Static properties of each output codec.
struct CodecDef {
    DdsCodec codec;
    const char* name;
    const char* alias; // legacy name, may be empty
    bool block;
    BlockFormat blockFormat;
    const char* fourcc;    // legacy header FourCC, "" for mask formats
    unsigned dxgi;         // DXGI_FORMAT, 0 = none
    unsigned dxgiSrgb;     // sRGB variant, 0 = none
    unsigned pfFlags;      // mask formats only
    unsigned bits;
    unsigned masks[4];     // r, g, b, a
};

const CodecDef kCodecs[] = {
    {DdsCodec::BC1, "bc1", "dxt1", true, BlockFormat::BC1, "DXT1", 71, 72, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC1A, "bc1a", "dxt1a", true, BlockFormat::BC1A, "DXT1", 71, 72, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC2, "bc2", "dxt3", true, BlockFormat::BC2, "DXT3", 74, 75, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC3, "bc3", "dxt5", true, BlockFormat::BC3, "DXT5", 77, 78, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC4, "bc4", "ati1", true, BlockFormat::BC4, "ATI1", 80, 0, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC5, "bc5", "ati2", true, BlockFormat::BC5, "ATI2", 83, 0, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::BC7, "bc7", "", true, BlockFormat::BC7, "", 98, 99, 0, 0, {0, 0, 0, 0}},
    {DdsCodec::RGBA8, "rgba8", "rgba", false, BlockFormat::BC1, "", 28, 29, DDPF_RGB | DDPF_ALPHAPIXELS, 32,
     {0xff, 0xff00, 0xff0000, 0xff000000u}},
    {DdsCodec::BGRA8, "bgra8", "bgra", false, BlockFormat::BC1, "", 87, 91, DDPF_RGB | DDPF_ALPHAPIXELS, 32,
     {0xff0000, 0xff00, 0xff, 0xff000000u}},
    {DdsCodec::BGRX8, "bgrx8", "bgrx", false, BlockFormat::BC1, "", 88, 93, DDPF_RGB, 32,
     {0xff0000, 0xff00, 0xff, 0}},
    {DdsCodec::BGR8, "bgr8", "bgr", false, BlockFormat::BC1, "", 0, 0, DDPF_RGB, 24, {0xff0000, 0xff00, 0xff, 0}},
    {DdsCodec::L8, "l8", "luminance", false, BlockFormat::BC1, "", 61, 0, DDPF_LUMINANCE, 8, {0xff, 0, 0, 0}},
    {DdsCodec::A8, "a8", "alpha", false, BlockFormat::BC1, "", 65, 0, DDPF_ALPHA, 8, {0, 0, 0, 0xff}},
};

const CodecDef& codecDef(DdsCodec c)
{
    for (size_t i = 0; i < sizeof(kCodecs) / sizeof(kCodecs[0]); ++i) {
        if (kCodecs[i].codec == c)
            return kCodecs[i];
    }
    throw Error("DDS: unresolved codec");
}

void put32(std::vector<unsigned char>& out, unsigned v)
{
    for (int i = 0; i < 4; ++i)
        out.push_back(static_cast<unsigned char>((v >> (8 * i)) & 0xff));
}

unsigned get32(const unsigned char* p)
{
    return static_cast<unsigned>(p[0]) | static_cast<unsigned>(p[1]) << 8 | static_cast<unsigned>(p[2]) << 16 |
           static_cast<unsigned>(p[3]) << 24;
}

// Packs a quantized RGBA8 surface into a mask-based layout.
void packUncompressed(const CodecDef& def, const Image& level, std::vector<unsigned char>& out)
{
    const unsigned bytes = def.bits / 8;
    for (size_t i = 0, n = level.pixelCount(); i < n; ++i) {
        const float* p = &level.data[i * 4];
        unsigned char rgba[4] = {toByte(p[0]), toByte(p[1]), toByte(p[2]), toByte(p[3])};
        unsigned v = 0;
        if (def.codec == DdsCodec::L8) {
            v = toByte(luma(p));
        } else {
            for (int c = 0; c < 4; ++c) {
                unsigned m = def.masks[c];
                if (!m)
                    continue;
                unsigned shift = 0;
                while (!((m >> shift) & 1))
                    ++shift;
                v |= static_cast<unsigned>(rgba[c]) << shift;
            }
        }
        for (unsigned b = 0; b < bytes; ++b)
            out.push_back(static_cast<unsigned char>((v >> (8 * b)) & 0xff));
    }
}

// Extracts a channel described by `mask` and expands it to 8 bits.
unsigned char extractMasked(unsigned v, unsigned mask)
{
    if (!mask)
        return 0;
    unsigned shift = 0;
    while (!((mask >> shift) & 1))
        ++shift;
    unsigned bits = 0;
    while ((mask >> (shift + bits)) & 1 && shift + bits < 32)
        ++bits;
    unsigned x = (v & mask) >> shift;
    unsigned maxv = bits >= 32 ? 0xffffffffu : (1u << bits) - 1;
    return static_cast<unsigned char>((static_cast<unsigned long long>(x) * 255 + maxv / 2) / maxv);
}

} // namespace

bool parseDdsCodec(const std::string& sIn, DdsCodec& out)
{
    std::string s = toLower(sIn);
    if (s == "auto") {
        out = DdsCodec::Auto;
        return true;
    }
    if (s == "dxt5n") // no special handling, same container
        s = "bc3";
    if (s == "bc4u")
        s = "bc4";
    if (s == "bc5u")
        s = "bc5";
    for (size_t i = 0; i < sizeof(kCodecs) / sizeof(kCodecs[0]); ++i) {
        if (s == kCodecs[i].name || s == kCodecs[i].alias) {
            out = kCodecs[i].codec;
            return true;
        }
    }
    return false;
}

std::string ddsCodecName(DdsCodec c)
{
    if (c == DdsCodec::Auto)
        return "auto";
    const CodecDef& d = codecDef(c);
    return *d.alias ? std::string(d.name) + "/" + d.alias : std::string(d.name);
}

std::vector<std::string> ddsCodecNames()
{
    std::vector<std::string> n;
    n.push_back("auto");
    for (size_t i = 0; i < sizeof(kCodecs) / sizeof(kCodecs[0]); ++i)
        n.push_back(ddsCodecName(kCodecs[i].codec));
    return n;
}

DdsResult encodeDds(const Image& img, const DdsOptions& opts, int jobs)
{
    if (img.empty())
        throw Error("DDS: empty image");

    DdsResult result;
    result.codec = opts.codec;
    if (result.codec == DdsCodec::Auto)
        result.codec = analyzeAlpha(img).opaque ? DdsCodec::BC1 : DdsCodec::BC3;
    const CodecDef& def = codecDef(result.codec);

    bool dx10 = opts.dx10 == 1 || (opts.dx10 == -1 && (result.codec == DdsCodec::BC7 || opts.srgb));
    if (dx10 && def.dxgi == 0)
        throw UsageError("DDS: codec " + std::string(def.name) + " has no DX10/DXGI equivalent (use dx10=0)");
    if (!dx10 && !*def.fourcc && def.pfFlags == 0)
        throw UsageError("DDS: codec " + std::string(def.name) + " requires the DX10 header (dx10=1)");
    if (opts.srgb && !dx10)
        throw UsageError("DDS: srgb=1 requires the DX10 header");
    if (opts.srgb && def.dxgiSrgb == 0)
        throw UsageError("DDS: codec " + std::string(def.name) + " has no sRGB variant");
    result.dx10 = dx10;

    std::vector<Image> chain = buildMipChain(img, opts.mips, jobs);
    result.mipLevels = static_cast<int>(chain.size());

    // --- header ---
    std::vector<unsigned char>& out = result.bytes;
    out.insert(out.end(), {'D', 'D', 'S', ' '});
    const bool mipped = chain.size() > 1;
    unsigned flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
    unsigned pitchOrSize;
    if (def.block) {
        flags |= DDSD_LINEARSIZE;
        pitchOrSize = static_cast<unsigned>(compressedSize(def.blockFormat, img.width, img.height));
    } else {
        flags |= DDSD_PITCH;
        pitchOrSize = static_cast<unsigned>((img.width * def.bits + 7) / 8);
    }
    if (mipped)
        flags |= DDSD_MIPMAPCOUNT;

    put32(out, 124);
    put32(out, flags);
    put32(out, static_cast<unsigned>(img.height));
    put32(out, static_cast<unsigned>(img.width));
    put32(out, pitchOrSize);
    put32(out, 0); // depth
    put32(out, mipped ? static_cast<unsigned>(chain.size()) : 0);
    for (int i = 0; i < 11; ++i)
        put32(out, 0); // reserved

    // DDS_PIXELFORMAT
    put32(out, 32);
    if (dx10) {
        put32(out, DDPF_FOURCC);
        put32(out, fourCC("DX10"));
        for (int i = 0; i < 5; ++i)
            put32(out, 0);
    } else if (def.block) {
        put32(out, DDPF_FOURCC | (result.codec == DdsCodec::BC1A ? DDPF_ALPHAPIXELS : 0));
        put32(out, fourCC(def.fourcc));
        for (int i = 0; i < 5; ++i)
            put32(out, 0);
    } else {
        put32(out, def.pfFlags);
        put32(out, 0);
        put32(out, def.bits);
        for (int c = 0; c < 4; ++c)
            put32(out, def.masks[c]);
    }

    put32(out, DDSCAPS_TEXTURE | (mipped ? DDSCAPS_COMPLEX | DDSCAPS_MIPMAP : 0));
    for (int i = 0; i < 4; ++i)
        put32(out, 0); // caps2..4, reserved2

    if (dx10) {
        put32(out, opts.srgb ? def.dxgiSrgb : def.dxgi);
        put32(out, DDS_RESOURCE_DIMENSION_TEXTURE2D);
        put32(out, 0); // misc flags
        put32(out, 1); // array size
        put32(out, 0); // alpha mode unknown
    }

    // --- surfaces ---
    for (size_t i = 0; i < chain.size(); ++i) {
        const Image& level = chain[i];
        if (def.block) {
            std::vector<unsigned char> rgba = toRGBA8(level);
            std::vector<unsigned char> blocks =
                compressSurface(&rgba[0], level.width, level.height, def.blockFormat, opts.block, jobs);
            out.insert(out.end(), blocks.begin(), blocks.end());
        } else {
            packUncompressed(def, level, out);
        }
    }
    return result;
}

Image decodeDds(const std::vector<unsigned char>& bytes, const std::string& name, std::string* formatDesc)
{
    if (bytes.size() < 128 || std::memcmp(&bytes[0], "DDS ", 4) != 0 || get32(&bytes[4]) != 124)
        throw Error("'" + name + "': not a valid DDS file");
    const unsigned char* h = &bytes[4];
    const int height = static_cast<int>(get32(h + 8));
    const int width = static_cast<int>(get32(h + 12));
    checkDimensions(width, height, name.c_str());
    const unsigned char* pf = h + 72;
    const unsigned pfFlags = get32(pf + 4);
    const unsigned fcc = get32(pf + 8);
    size_t offset = 128;

    bool block = false;
    BlockFormat bf = BlockFormat::BC1;
    bool premultiplied = false;
    unsigned bits = 0, masks[4] = {0, 0, 0, 0};
    unsigned layoutFlags = pfFlags;
    std::string desc;

    if (pfFlags & DDPF_FOURCC) {
        if (fcc == fourCC("DX10")) {
            if (bytes.size() < 148)
                throw Error("'" + name + "': truncated DX10 header");
            const unsigned dxgi = get32(&bytes[128]);
            offset = 148;
            desc = strprintf("DX10 format %u", dxgi);
            switch (dxgi) {
            case 70: case 71: case 72: block = true; bf = BlockFormat::BC1A; desc = "BC1"; break;
            case 73: case 74: case 75: block = true; bf = BlockFormat::BC2; desc = "BC2"; break;
            case 76: case 77: case 78: block = true; bf = BlockFormat::BC3; desc = "BC3"; break;
            case 79: case 80: block = true; bf = BlockFormat::BC4; desc = "BC4"; break;
            case 82: case 83: block = true; bf = BlockFormat::BC5; desc = "BC5"; break;
            case 97: case 98: case 99: block = true; bf = BlockFormat::BC7; desc = "BC7"; break;
            case 27: case 28: case 29:
                bits = 32; layoutFlags = DDPF_RGB | DDPF_ALPHAPIXELS;
                masks[0] = 0xff; masks[1] = 0xff00; masks[2] = 0xff0000; masks[3] = 0xff000000u;
                desc = "RGBA8"; break;
            case 87: case 90: case 91:
                bits = 32; layoutFlags = DDPF_RGB | DDPF_ALPHAPIXELS;
                masks[0] = 0xff0000; masks[1] = 0xff00; masks[2] = 0xff; masks[3] = 0xff000000u;
                desc = "BGRA8"; break;
            case 88: case 92: case 93:
                bits = 32; layoutFlags = DDPF_RGB;
                masks[0] = 0xff0000; masks[1] = 0xff00; masks[2] = 0xff;
                desc = "BGRX8"; break;
            case 61:
                bits = 8; layoutFlags = DDPF_LUMINANCE; masks[0] = 0xff; desc = "R8"; break;
            case 65:
                bits = 8; layoutFlags = DDPF_ALPHA; masks[3] = 0xff; desc = "A8"; break;
            default:
                throw Error("'" + name + "': unsupported DXGI format " + strprintf("%u", dxgi));
            }
        } else if (fcc == fourCC("DXT1")) {
            block = true; bf = BlockFormat::BC1A; desc = "DXT1";
        } else if (fcc == fourCC("DXT2") || fcc == fourCC("DXT3")) {
            block = true; bf = BlockFormat::BC2; premultiplied = fcc == fourCC("DXT2");
            desc = premultiplied ? "DXT2" : "DXT3";
        } else if (fcc == fourCC("DXT4") || fcc == fourCC("DXT5")) {
            block = true; bf = BlockFormat::BC3; premultiplied = fcc == fourCC("DXT4");
            desc = premultiplied ? "DXT4" : "DXT5";
        } else if (fcc == fourCC("ATI1") || fcc == fourCC("BC4U")) {
            block = true; bf = BlockFormat::BC4; desc = "BC4";
        } else if (fcc == fourCC("ATI2") || fcc == fourCC("BC5U")) {
            block = true; bf = BlockFormat::BC5; desc = "BC5";
        } else {
            char c[5] = {static_cast<char>(fcc & 0xff), static_cast<char>((fcc >> 8) & 0xff),
                         static_cast<char>((fcc >> 16) & 0xff), static_cast<char>(fcc >> 24), 0};
            throw Error("'" + name + "': unsupported DDS FourCC '" + c + "'");
        }
    } else {
        bits = get32(pf + 12);
        for (int c = 0; c < 4; ++c)
            masks[c] = get32(pf + 16 + 4 * c);
        if (bits != 8 && bits != 16 && bits != 24 && bits != 32)
            throw Error("'" + name + "': unsupported DDS bit depth " + strprintf("%u", bits));
        desc = strprintf("%u-bit uncompressed", bits);
    }

    std::vector<unsigned char> rgba;
    if (block) {
        size_t need = compressedSize(bf, width, height);
        if (bytes.size() < offset + need)
            throw Error("'" + name + "': truncated DDS data");
        rgba = decompressSurface(&bytes[offset], need, width, height, bf);
    } else {
        const size_t bpp = bits / 8;
        const size_t need = static_cast<size_t>(width) * height * bpp;
        if (bytes.size() < offset + need)
            throw Error("'" + name + "': truncated DDS data");
        rgba.resize(static_cast<size_t>(width) * height * 4);
        for (size_t i = 0, n = static_cast<size_t>(width) * height; i < n; ++i) {
            unsigned v = 0;
            for (size_t b = 0; b < bpp; ++b)
                v |= static_cast<unsigned>(bytes[offset + i * bpp + b]) << (8 * b);
            unsigned char* d = &rgba[i * 4];
            if (layoutFlags & DDPF_LUMINANCE) {
                d[0] = d[1] = d[2] = extractMasked(v, masks[0]);
            } else if ((layoutFlags & DDPF_ALPHA) && !(layoutFlags & DDPF_RGB)) {
                d[0] = d[1] = d[2] = 0;
            } else {
                for (int c = 0; c < 3; ++c)
                    d[c] = extractMasked(v, masks[c]);
            }
            bool hasAlpha = (layoutFlags & (DDPF_ALPHAPIXELS | DDPF_ALPHA)) && masks[3];
            d[3] = hasAlpha ? extractMasked(v, masks[3]) : 255;
        }
    }

    Image img = fromRGBA8(&rgba[0], width, height);
    if (premultiplied)
        unpremultiply(img);
    if (formatDesc)
        *formatDesc = desc;
    return img;
}

} // namespace img
} // namespace idt3
