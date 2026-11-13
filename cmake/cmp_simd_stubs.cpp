// Stand-ins for CMP_Core's x86 SIMD endpoint search. The BC1 encoder
// references them from its runtime dispatcher, but the replacement CPU
// detection (cmp_cpu_extensions.cpp) never selects them, so they only need
// to satisfy the linker.

#include <cstdio>
#include <cstdlib>

namespace {

[[noreturn]] float unavailable(const char* name)
{
    std::fprintf(stderr, "internal error: CMP_Core SIMD routine %s called although SIMD is disabled\n", name);
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
