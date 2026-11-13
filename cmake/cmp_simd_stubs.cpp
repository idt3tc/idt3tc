// Stand-ins for CMP_Core's x86 SIMD endpoint search on other
// architectures. The BC1 encoder references them from its runtime
// dispatcher, but the CPU detection never selects them there, so they
// only need to satisfy the linker.

#include <cstdio>
#include <cstdlib>

namespace {

[[noreturn]] float unavailable(const char* name)
{
    std::fprintf(stderr, "internal error: CMP_Core SIMD routine %s called on a non-x86 build\n", name);
    std::abort();
}

} // namespace

float sse_bc1ComputeBestEndpoints(float*, float*, float*, float*, float*, int, int)
{
    return unavailable("sse_bc1ComputeBestEndpoints");
}

float avx_bc1ComputeBestEndpoints(float*, float*, float*, float*, float*, int, int)
{
    return unavailable("avx_bc1ComputeBestEndpoints");
}

float avx512_bc1ComputeBestEndpoints(float*, float*, float*, float*, float*, int, int)
{
    return unavailable("avx512_bc1ComputeBestEndpoints");
}
