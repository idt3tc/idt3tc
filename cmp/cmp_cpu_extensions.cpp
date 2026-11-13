// Replacement for Compressonator's applications/_libs/cmp_math/cpu_extensions.cpp.
//
// Upstream detects SSE4.2/AVX2/AVX-512 with CPUID and lets the BC1 encoder
// dispatch its endpoint search to hand-vectorised variants. That detection
// only works with MSVC's __cpuidex: on Linux it is compiled out (always
// scalar), and on every other non-Windows target it reads an uninitialised
// buffer, so the encoder could pick an AVX-512 routine on a CPU without it,
// or one of the abort stubs on arm64.
//
// Reporting no extensions anywhere makes every build use the portable scalar
// code. That is what Linux builds always did, it lets the SIMD variants be
// left out of the build entirely, and it means a given input encodes to the
// same bytes on every platform and CPU.

#include "cpu_extensions.h"
#include "cmp_math_common.h"

void GetCPUID(int outInfo[4], int /*functionID*/)
{
    outInfo[0] = outInfo[1] = outInfo[2] = outInfo[3] = 0;
}

CPUExtensions GetCPUExtensions()
{
    CPUExtensions result = {};
    return result;
}

bool IsAvailableSSE4(CPUExtensions /*extensions*/)
{
    return false;
}

bool IsAvailableAVX2(CPUExtensions /*extensions*/)
{
    return false;
}

bool IsAvailableAVX512(CPUExtensions /*extensions*/)
{
    return false;
}

void cmp_autodetected_cpufeatures(CMP_MATH_BYTE /*set*/)
{
    cmp_set_cpu_features();
}
