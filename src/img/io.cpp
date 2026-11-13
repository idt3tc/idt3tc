#include "img/io.h"

#include "common/fs.h"
#include "common/util.h"

#include <stb_image.h>
#include <stb_image_write.h>

#include <cstring>

namespace idt3 {
namespace img {

namespace {

const char* const kImageExtensions[] = {"png", "jpg", "jpeg", "tga", "bmp", "psd", "gif",
                                        "hdr", "pic", "pnm", "ppm", "pgm", "dds"};

// Guess the container from magic bytes (stb_image does its own probing).
std::string sniffFormat(const std::vector<unsigned char>& b, const std::string& name)
{
    if (b.size() >= 8 && std::memcmp(&b[0], "\x89PNG", 4) == 0)
        return "png";
    if (b.size() >= 3 && b[0] == 0xff && b[1] == 0xd8)
        return "jpeg";
    if (b.size() >= 2 && b[0] == 'B' && b[1] == 'M')
        return "bmp";
    if (b.size() >= 4 && std::memcmp(&b[0], "8BPS", 4) == 0)
        return "psd";
    if (b.size() >= 4 && std::memcmp(&b[0], "GIF8", 4) == 0)
        return "gif";
    std::string ext = fs::extension(name);
    return ext.empty() ? std::string("image") : ext;
}

void appendBytes(void* context, void* data, int size)
{
    std::vector<unsigned char>* out = static_cast<std::vector<unsigned char>*>(context);
    const unsigned char* p = static_cast<const unsigned char*>(data);
    out->insert(out->end(), p, p + size);
}

// Interleaved 8-bit buffer with `channels` channels (1: luma, 2: luma +
// alpha, 3: RGB, 4: RGBA).
std::vector<unsigned char> packChannels(const Image& img, int channels)
{
    std::vector<unsigned char> out(img.pixelCount() * static_cast<size_t>(channels));
    for (size_t i = 0, n = img.pixelCount(); i < n; ++i) {
        const float* p = &img.data[i * 4];
        unsigned char* d = &out[i * channels];
        if (channels <= 2) {
            d[0] = toByte(luma(p));
            if (channels == 2)
                d[1] = toByte(p[3]);
        } else {
            for (int c = 0; c < channels; ++c)
                d[c] = toByte(p[c]);
        }
    }
    return out;
}

int autoChannels(const Image& img, int channels)
{
    if (channels > 0)
        return channels;
    return analyzeAlpha(img).opaque ? 3 : 4;
}

} // namespace

bool isImageExtension(const std::string& ext)
{
    for (size_t i = 0; i < sizeof(kImageExtensions) / sizeof(kImageExtensions[0]); ++i) {
        if (ext == kImageExtensions[i])
            return true;
    }
    return false;
}

Image decodeImage(const std::vector<unsigned char>& bytes, const std::string& name, LoadInfo* info)
{
    if (bytes.size() >= 4 && std::memcmp(&bytes[0], "DDS ", 4) == 0) {
        std::string desc;
        Image img = decodeDds(bytes, name, &desc);
        if (info) {
            info->format = "dds (" + desc + ")";
            info->channels = 4;
            info->bitsPerChannel = 8;
        }
        return img;
    }

    if (bytes.size() > 0x7fffffff)
        throw Error("'" + name + "': file too large");
    const stbi_uc* buf = bytes.empty() ? nullptr : &bytes[0];
    const int len = static_cast<int>(bytes.size());
    int w = 0, h = 0, comp = 0;
    if (!stbi_info_from_memory(buf, len, &w, &h, &comp))
        throw Error("'" + name + "': unsupported or corrupt image (" + stbi_failure_reason() + ")");
    checkDimensions(w, h, name.c_str());

    Image img;
    int bits = 8;
    if (stbi_is_16_bit_from_memory(buf, len)) {
        // Keep the extra precision of 16-bit PNG/PSD sources.
        stbi_us* px = stbi_load_16_from_memory(buf, len, &w, &h, &comp, 4);
        if (!px)
            throw Error("'" + name + "': decoding failed (" + stbi_failure_reason() + ")");
        img.allocate(w, h);
        for (size_t i = 0; i < img.data.size(); ++i)
            img.data[i] = px[i] / 65535.0f;
        stbi_image_free(px);
        bits = 16;
    } else {
        stbi_uc* px = stbi_load_from_memory(buf, len, &w, &h, &comp, 4);
        if (!px)
            throw Error("'" + name + "': decoding failed (" + stbi_failure_reason() + ")");
        img = fromRGBA8(px, w, h);
        stbi_image_free(px);
    }
    if (info) {
        info->format = sniffFormat(bytes, name);
        info->channels = comp;
        info->bitsPerChannel = bits;
    }
    return img;
}

Image loadImage(const std::string& path, LoadInfo* info)
{
    return decodeImage(fs::readFile(path), path, info);
}

std::vector<unsigned char> encodePng(const Image& img, int channels, int compressionLevel)
{
    channels = autoChannels(img, channels);
    std::vector<unsigned char> px = packChannels(img, channels);
    std::vector<unsigned char> out;
    // stb exposes the zlib level only as a global; writes are serialized
    // per process run so this is safe for the CLI.
    stbi_write_png_compression_level = compressionLevel;
    if (!stbi_write_png_to_func(appendBytes, &out, img.width, img.height, channels, &px[0], img.width * channels))
        throw Error("PNG encoding failed");
    return out;
}

std::vector<unsigned char> encodeJpg(const Image& img, int quality)
{
    // JPEG has no alpha channel.
    std::vector<unsigned char> px = packChannels(img, 3);
    std::vector<unsigned char> out;
    if (!stbi_write_jpg_to_func(appendBytes, &out, img.width, img.height, 3, &px[0], quality))
        throw Error("JPEG encoding failed");
    return out;
}

std::vector<unsigned char> encodeBmp(const Image& img, int channels)
{
    channels = autoChannels(img, channels);
    if (channels != 3 && channels != 4)
        throw UsageError("BMP output supports 3 or 4 channels only");
    std::vector<unsigned char> px = packChannels(img, channels);
    std::vector<unsigned char> out;
    if (!stbi_write_bmp_to_func(appendBytes, &out, img.width, img.height, channels, &px[0]))
        throw Error("BMP encoding failed");
    return out;
}

} // namespace img
} // namespace idt3
