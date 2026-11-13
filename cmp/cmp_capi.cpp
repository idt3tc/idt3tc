// C linkage wrappers around the CMP_Core block API used by the Rust code.
// Upstream's cmp_core.h declares its functions without extern "C", so they
// carry C++ mangled names that differ between compilers; these wrappers give
// Rust a stable ABI. The codec is selected by its BCn number (1..5, 7).

// cmp_core.h uses NULL without including the header that defines it.
#include <cstddef>

#include "cmp_core.h"

// Defined in bc7_encode_kernel.cpp without a declaration in cmp_core.h.
void init_BC7ramps();

namespace {

// CMP_Core initializes some global state lazily on first use and is not
// thread safe while doing so: init_BC7ramps() sets its "done" flag before
// it fills the BC7 ramp tables, and the BC1 encoder publishes its endpoint
// search function pointer through an unsynchronized flag. Blocks encoded
// concurrently on a fresh process could therefore read half-built tables
// (a data race that produced thread count dependent BC7 output). All
// lazy initialization is done here exactly once, under the thread-safe
// initialization of a function-local static, before any wrapper touches
// the codecs.
bool init_once()
{
    // BC7 ramp tables (also built by CreateOptionsBC7 and the decoder).
    init_BC7ramps();
    // The BC1 encoder selects its endpoint search on the first block.
    void* bc1 = nullptr;
    // 0 is CGU_CORE_OK, which cmp_core.h does not expose.
    if (CreateOptionsBC1(&bc1) != 0)
        return false;
    unsigned char block[64] = {};
    unsigned char out[8];
    for (int i = 0; i < 64; i++)
        block[i] = static_cast<unsigned char>(i * 37);
    int rc = CompressBlockBC1(block, 16, out, bc1);
    DestroyOptionsBC1(bc1);
    return rc == 0;
}

bool ensure_init()
{
    static const bool ok = init_once();
    return ok;
}

} // namespace

extern "C" {

int idt3tc_cmp_create_options(int bc, void** options) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 1: return CreateOptionsBC1(options);
    case 2: return CreateOptionsBC2(options);
    case 3: return CreateOptionsBC3(options);
    case 4: return CreateOptionsBC4(options);
    case 5: return CreateOptionsBC5(options);
    case 7: return CreateOptionsBC7(options);
    default: return -1;
    }
}

int idt3tc_cmp_destroy_options(int bc, void* options) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 1: return DestroyOptionsBC1(options);
    case 2: return DestroyOptionsBC2(options);
    case 3: return DestroyOptionsBC3(options);
    case 4: return DestroyOptionsBC4(options);
    case 5: return DestroyOptionsBC5(options);
    case 7: return DestroyOptionsBC7(options);
    default: return -1;
    }
}

int idt3tc_cmp_set_quality(int bc, void* options, float quality) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 1: return SetQualityBC1(options, quality);
    case 2: return SetQualityBC2(options, quality);
    case 3: return SetQualityBC3(options, quality);
    case 4: return SetQualityBC4(options, quality);
    case 5: return SetQualityBC5(options, quality);
    case 7: return SetQualityBC7(options, quality);
    default: return -1;
    }
}

// Only BC1..BC3 have a perceptual color error metric.
int idt3tc_cmp_set_channel_weights(int bc, void* options, float r, float g, float b) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 1: return SetChannelWeightsBC1(options, r, g, b);
    case 2: return SetChannelWeightsBC2(options, r, g, b);
    case 3: return SetChannelWeightsBC3(options, r, g, b);
    default: return -1;
    }
}

int idt3tc_cmp_set_refine_steps_bc1(void* options, unsigned int steps) noexcept
{
    if (!ensure_init())
        return -1;
    return SetRefineStepsBC1(options, steps);
}

// `rgba` is a complete 4x4 RGBA8 block (64 bytes, rows 16 bytes apart);
// `out` receives 8 (BC1) or 16 bytes.
int idt3tc_cmp_compress_rgba(int bc, const unsigned char* rgba, unsigned char* out, const void* options) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 1: return CompressBlockBC1(rgba, 16, out, options);
    case 2: return CompressBlockBC2(rgba, 16, out, options);
    case 3: return CompressBlockBC3(rgba, 16, out, options);
    case 7: return CompressBlockBC7(rgba, 16, out, options);
    default: return -1;
    }
}

// Single channel 4x4 block (16 bytes) to 8 bytes of BC4.
int idt3tc_cmp_compress_bc4(const unsigned char* r, unsigned char* out, const void* options) noexcept
{
    if (!ensure_init())
        return -1;
    return CompressBlockBC4(r, 4, out, options);
}

// Two 4x4 channel blocks (16 bytes each) to 16 bytes of BC5.
int idt3tc_cmp_compress_bc5(const unsigned char* r, const unsigned char* g, unsigned char* out,
                            const void* options) noexcept
{
    if (!ensure_init())
        return -1;
    return CompressBlockBC5(r, 4, g, 4, out, options);
}

// 16 bytes of BC2/BC3/BC7 to a 64 byte RGBA8 block.
int idt3tc_cmp_decompress_rgba(int bc, const unsigned char* in, unsigned char* rgba) noexcept
{
    if (!ensure_init())
        return -1;
    switch (bc) {
    case 2: return DecompressBlockBC2(in, rgba, nullptr);
    case 3: return DecompressBlockBC3(in, rgba, nullptr);
    case 7: return DecompressBlockBC7(in, rgba, nullptr);
    default: return -1;
    }
}

int idt3tc_cmp_decompress_bc4(const unsigned char* in, unsigned char* r) noexcept
{
    if (!ensure_init())
        return -1;
    return DecompressBlockBC4(in, r, nullptr);
}

int idt3tc_cmp_decompress_bc5(const unsigned char* in, unsigned char* r, unsigned char* g) noexcept
{
    if (!ensure_init())
        return -1;
    return DecompressBlockBC5(in, r, g, nullptr);
}

} // extern "C"
