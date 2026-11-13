// File format encoders/decoders and block compression.

#include "helpers.h"
#include "test.h"

#include "common/util.h"
#include "img/bcn.h"
#include "img/io.h"

using namespace idt3;
using namespace idt3::img;
using idt3test::makeTestImage;
using idt3test::maxByteDiff;
using idt3test::psnr;

namespace {

unsigned readLE32(const std::vector<unsigned char>& b, size_t off)
{
    return b[off] | (b[off + 1] << 8) | (b[off + 2] << 16) | (unsigned(b[off + 3]) << 24);
}

Image ddsRoundtrip(const Image& src, DdsCodec codec, DdsResult* result = nullptr, int dx10 = -1)
{
    DdsOptions o;
    o.codec = codec;
    o.dx10 = dx10;
    DdsResult r = encodeDds(src, o, 4);
    std::string desc;
    Image out = decodeDds(r.bytes, "test.dds", &desc);
    if (result)
        *result = r;
    return out;
}

} // namespace

TEST(tga_roundtrip_variants)
{
    Image src = makeTestImage(33, 17, true);
    for (int rle = 0; rle < 2; ++rle) {
        for (int top = 0; top < 2; ++top) {
            TgaOptions o;
            o.rle = rle != 0;
            o.topOrigin = top != 0;
            int bpp = 0;
            std::vector<unsigned char> bytes = encodeTga(src, o, &bpp);
            CHECK_EQ(bpp, 32);
            CHECK_EQ(int(bytes[2]), rle ? 10 : 2);
            CHECK_EQ(int(bytes[17] & 0x0f), 8);              // alpha bits
            CHECK_EQ(int((bytes[17] & 0x20) != 0), top);      // origin flag
            Image back = decodeImage(bytes, "t.tga");
            CHECK_EQ(back.width, 33);
            CHECK_EQ(back.height, 17);
            CHECK_EQ(maxByteDiff(src, back), 0);
        }
    }
}

TEST(tga_24_and_8_bit)
{
    Image src = makeTestImage(16, 8, false);
    int bpp = 0;
    std::vector<unsigned char> b24 = encodeTga(src, TgaOptions(), &bpp);
    CHECK_EQ(bpp, 24);
    CHECK_EQ(b24.size(), size_t(18 + 16 * 8 * 3));
    CHECK_EQ(maxByteDiff(src, decodeImage(b24, "t.tga")), 0);

    Image gray(4, 4, Color(0.5f, 0.5f, 0.5f, 1));
    TgaOptions o8;
    o8.bpp = 8;
    std::vector<unsigned char> b8 = encodeTga(gray, o8, &bpp);
    CHECK_EQ(bpp, 8);
    CHECK_EQ(int(b8[2]), 3);
    CHECK_EQ(maxByteDiff(gray, decodeImage(b8, "t.tga")), 0);
}

TEST(tga_rle_runs_stay_in_scanline)
{
    // A uniform image compresses to one run per row (idTech 3 decodes RLE
    // row by row and rejects packets crossing a scanline).
    Image src(200, 3, Color(1, 0, 0, 1));
    TgaOptions o;
    o.rle = true;
    std::vector<unsigned char> b = encodeTga(src, o);
    // header + per row: a run of 128 and a run of 72, each 1 + 3 bytes
    CHECK_EQ(b.size(), size_t(18 + 3 * 8));
    CHECK_EQ(maxByteDiff(src, decodeImage(b, "t.tga")), 0);
}

TEST(dds_bc_quality)
{
    Image rgb = makeTestImage(64, 48, false);
    Image rgba = makeTestImage(64, 48, true);
    struct Case {
        DdsCodec codec;
        const Image* img;
        int mask;
        double minPsnr;
    } cases[] = {
        {DdsCodec::BC1, &rgb, 7, 30.0},  {DdsCodec::BC2, &rgba, 15, 28.0}, {DdsCodec::BC3, &rgba, 15, 30.0},
        {DdsCodec::BC4, &rgb, 1, 38.0},  {DdsCodec::BC5, &rgb, 3, 36.0},   {DdsCodec::BC7, &rgba, 15, 38.0},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        DdsResult r;
        Image out = ddsRoundtrip(*cases[i].img, cases[i].codec, &r);
        CHECK_EQ(out.width, 64);
        CHECK_EQ(out.height, 48);
        double p = psnr(*cases[i].img, out, cases[i].mask);
        if (p < cases[i].minPsnr)
            idt3test::fail(__FILE__, __LINE__,
                           strprintf("%s: PSNR %.2f dB < %.1f", ddsCodecName(cases[i].codec).c_str(), p,
                                     cases[i].minPsnr));
    }
}

TEST(dds_uncompressed_exact)
{
    Image src = makeTestImage(13, 7, true);
    DdsCodec codecs[] = {DdsCodec::RGBA8, DdsCodec::BGRA8};
    for (size_t i = 0; i < 2; ++i)
        CHECK_EQ(maxByteDiff(src, ddsRoundtrip(src, codecs[i])), 0);
    Image opaque = makeTestImage(13, 7, false);
    CHECK_EQ(maxByteDiff(opaque, ddsRoundtrip(opaque, DdsCodec::BGR8)), 0);
    CHECK_EQ(maxByteDiff(opaque, ddsRoundtrip(opaque, DdsCodec::BGRX8)), 0);
}

TEST(dds_bc1a_binary_alpha)
{
    Image src = makeTestImage(32, 32, false);
    for (int y = 0; y < 32; ++y) {
        for (int x = 0; x < 32; ++x)
            src.pixel(x, y)[3] = ((x / 3 + y / 5) % 2) ? 1.0f : 0.0f;
    }
    Image out = ddsRoundtrip(src, DdsCodec::BC1A);
    for (size_t i = 3; i < src.data.size(); i += 4)
        CHECK_EQ(toByte(out.data[i]), toByte(src.data[i]));
}

TEST(dds_bc1_opaque_never_transparent)
{
    // Content that tempts an encoder into 3-color mode (black texels).
    Image src = makeTestImage(32, 32, false);
    for (int y = 0; y < 32; y += 2) {
        for (int x = 0; x < 32; x += 3) {
            float* p = src.pixel(x, y);
            p[0] = p[1] = p[2] = 0.0f;
        }
    }
    Image out = ddsRoundtrip(src, DdsCodec::BC1);
    for (size_t i = 3; i < out.data.size(); i += 4)
        CHECK_EQ(toByte(out.data[i]), 255);
}

TEST(dds_auto_codec_and_headers)
{
    DdsResult r;
    ddsRoundtrip(makeTestImage(64, 32, false), DdsCodec::Auto, &r);
    CHECK(r.codec == DdsCodec::BC1);
    CHECK_EQ(r.mipLevels, 7);
    CHECK_EQ(readLE32(r.bytes, 0), 0x20534444u); // "DDS "
    CHECK_EQ(readLE32(r.bytes, 28), 7u);          // dwMipMapCount
    CHECK_EQ(readLE32(r.bytes, 84), 0x31545844u); // "DXT1"
    // 64x32 BC1 chain: 1024+256+64+16+8+8+8 bytes (blocks are at least 4x4)
    CHECK_EQ(r.bytes.size(), size_t(128 + 1024 + 256 + 64 + 16 + 8 + 8 + 8));

    ddsRoundtrip(makeTestImage(16, 16, true), DdsCodec::Auto, &r);
    CHECK(r.codec == DdsCodec::BC3);

    ddsRoundtrip(makeTestImage(16, 16, true), DdsCodec::BC7, &r);
    CHECK(r.dx10);
    CHECK_EQ(readLE32(r.bytes, 84), 0x30315844u); // "DX10"
    CHECK_EQ(readLE32(r.bytes, 128), 98u);        // DXGI_FORMAT_BC7_UNORM
}

TEST(dds_mip_options)
{
    DdsOptions o;
    o.codec = DdsCodec::BC3;
    o.mips.levels = 1;
    CHECK_EQ(encodeDds(makeTestImage(64, 64, true), o, 2).mipLevels, 1);
    o.mips.levels = 0;
    o.mips.minSize = 8;
    CHECK_EQ(encodeDds(makeTestImage(64, 64, true), o, 2).mipLevels, 4);
}

TEST(dds_invalid_combinations)
{
    DdsOptions o;
    o.codec = DdsCodec::BC7;
    o.dx10 = 0;
    CHECK_THROWS(encodeDds(makeTestImage(8, 8, true), o, 1), UsageError);
    o.codec = DdsCodec::BC4;
    o.dx10 = -1;
    o.srgb = true;
    CHECK_THROWS(encodeDds(makeTestImage(8, 8, true), o, 1), UsageError);
}

TEST(bcn_threads_match)
{
    Image src = makeTestImage(70, 45, true);
    std::vector<unsigned char> rgba = toRGBA8(src);
    BlockFormat formats[] = {BlockFormat::BC1, BlockFormat::BC3, BlockFormat::BC5, BlockFormat::BC7};
    for (size_t i = 0; i < 4; ++i) {
        BlockOptions o;
        o.quality = 0.2f;
        std::vector<unsigned char> a = compressSurface(rgba.data(), 70, 45, formats[i], o, 1);
        std::vector<unsigned char> b = compressSurface(rgba.data(), 70, 45, formats[i], o, 8);
        CHECK(a == b);
        CHECK_EQ(a.size(), compressedSize(formats[i], 70, 45));
    }
}

TEST(png_jpg_bmp_roundtrip)
{
    Image src = makeTestImage(31, 9, true);
    CHECK_EQ(maxByteDiff(src, decodeImage(encodePng(src, 0, 6), "t.png")), 0);
    Image opaque = makeTestImage(31, 9, false);
    CHECK_EQ(maxByteDiff(opaque, decodeImage(encodeBmp(opaque, 0), "t.bmp")), 0);
    CHECK(psnr(opaque, decodeImage(encodeJpg(opaque, 95), "t.jpg"), 7) > 30.0);
}
