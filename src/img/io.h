#pragma once

#include "img/bcn.h"
#include "img/image.h"
#include "img/mipmap.h"

#include <string>
#include <vector>

namespace idt3 {
namespace img {

// --- Loading ----------------------------------------------------------------

struct LoadInfo {
    std::string format; // "png", "tga", "dds (DXT5)", ...
    int channels;       // channels stored in the file
    int bitsPerChannel; // 8 or 16
};

// Loads PNG, JPEG, TGA, BMP, PSD, GIF, HDR, PIC, PNM (via stb_image) and
// DDS (BC1-5, BC7 and common uncompressed formats; top mip only).
Image loadImage(const std::string& path, LoadInfo* info = nullptr);
Image decodeImage(const std::vector<unsigned char>& bytes, const std::string& name, LoadInfo* info = nullptr);

// Extensions (lowercase, no dot) treated as images when scanning folders.
bool isImageExtension(const std::string& ext);

// --- TGA --------------------------------------------------------------------

// Defaults match what the idTech 3 TGA loader handles: uncompressed,
// bottom-up rows (ioquake3 ignores the top-down flag and would show the
// texture upside down).
struct TgaOptions {
    int bpp;        // 0 = auto (32 with alpha, else 24), 8 (gray), 24, 32
    bool rle;       // RLE compression (image types 10 and 11)
    bool topOrigin; // rows stored top-down (descriptor bit 5)
    TgaOptions() : bpp(0), rle(false), topOrigin(false) {}
};

// Returns the file bytes; `bppUsed` receives the resolved bit depth.
std::vector<unsigned char> encodeTga(const Image& img, const TgaOptions& opts, int* bppUsed = nullptr);

// --- DDS --------------------------------------------------------------------

enum class DdsCodec { Auto, BC1, BC1A, BC2, BC3, BC4, BC5, BC7, RGBA8, BGRA8, BGRX8, BGR8, L8, A8 };

bool parseDdsCodec(const std::string& s, DdsCodec& out);
std::string ddsCodecName(DdsCodec c);
std::vector<std::string> ddsCodecNames();

struct DdsOptions {
    DdsCodec codec;
    MipOptions mips;     // mips.levels == 1 disables mipmaps
    BlockOptions block;
    int dx10;            // -1 auto (only when required), 0 never, 1 always
    bool srgb;           // tag as sRGB (DX10 header only)
    DdsOptions() : codec(DdsCodec::Auto), dx10(-1), srgb(false) {}
};

struct DdsResult {
    std::vector<unsigned char> bytes;
    DdsCodec codec; // resolved codec
    int mipLevels;
    bool dx10;
};

DdsResult encodeDds(const Image& img, const DdsOptions& opts, int jobs);

// Decodes the top mip level of a DDS file.
Image decodeDds(const std::vector<unsigned char>& bytes, const std::string& name, std::string* formatDesc);

// --- PNG / JPEG / BMP (stb_image_write) -------------------------------------

// channels: 0 = auto (4 if the image has alpha, else 3), or 1..4.
std::vector<unsigned char> encodePng(const Image& img, int channels, int compressionLevel);
std::vector<unsigned char> encodeJpg(const Image& img, int quality);
std::vector<unsigned char> encodeBmp(const Image& img, int channels);

} // namespace img
} // namespace idt3
