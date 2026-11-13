#pragma once

#include <cstddef>
#include <string>
#include <vector>

// Block compression (BCn / S3TC) on top of Compressonator's CMP_Core.
namespace idt3 {
namespace img {

enum class BlockFormat {
    BC1,  // DXT1, opaque RGB
    BC1A, // DXT1 with 1-bit (punch-through) alpha
    BC2,  // DXT3, explicit 4-bit alpha
    BC3,  // DXT5, interpolated alpha
    BC4,  // ATI1, single channel (red)
    BC5,  // ATI2, two channels (red, green), e.g. normal maps
    BC7   // high quality RGBA, requires the DX10 DDS header
};

// Bytes per 4x4 block.
size_t blockBytes(BlockFormat f);

// Size of the compressed data for one w x h surface.
size_t compressedSize(BlockFormat f, int w, int h);

struct BlockOptions {
    float quality;         // 0..1, Compressonator's speed/quality trade-off
    int refineSteps;       // BC1/BC1A endpoint refinement iterations (0..)
    bool useWeights;       // use `weights` for color error metric (BC1-3)
    float weights[3];      // perceptual channel weights (r, g, b)
    int alphaThreshold;    // BC1A: alpha below this becomes transparent
    BlockOptions() : quality(1.0f), refineSteps(0), useWeights(false), alphaThreshold(128)
    {
        weights[0] = 0.3086f;
        weights[1] = 0.6094f;
        weights[2] = 0.0820f;
    }
};

// Compresses a tightly packed RGBA8 surface. Partial edge blocks are padded
// by replicating the last row/column.
std::vector<unsigned char> compressSurface(const unsigned char* rgba, int w, int h, BlockFormat f,
                                           const BlockOptions& opts, int jobs);

// Decompresses one surface to tightly packed RGBA8. BC4 is expanded to
// gray, BC5 to (r, g, 0).
std::vector<unsigned char> decompressSurface(const unsigned char* data, size_t size, int w, int h,
                                             BlockFormat f);

} // namespace img
} // namespace idt3
