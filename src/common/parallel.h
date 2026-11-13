#pragma once

#include <functional>

namespace idt3 {

// Number of hardware threads (at least 1).
int hardwareJobs();

// Splits [0, count) into contiguous chunks and runs fn(begin, end) on up to
// `jobs` threads. The first exception thrown by any chunk is rethrown on the
// calling thread after all workers have finished.
void parallelFor(int count, int jobs, const std::function<void(int, int)>& fn);

} // namespace idt3
