// Force-included into CMP_Core when targeting x86 Windows with clang.
//
// CMP_Core's cmp_floor() calls _mm_floor_ss(v, v) on _M_X64/_M_IX86, an
// SSE4.1 instruction that clang only allows when compiling for SSE4.1. The
// replacement below is exact for that call (v holds the value in lane 0 and
// zeros elsewhere), only uses SSE, and gives the same result as the floorf()
// every other platform uses.

#pragma once

#include <math.h>
#include <smmintrin.h>

#undef _mm_floor_ss
#define _mm_floor_ss(a, b) _mm_set_ss(floorf(_mm_cvtss_f32(b)))
