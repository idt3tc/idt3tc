#include "common/parallel.h"

#include <algorithm>
#include <atomic>
#include <exception>
#include <mutex>
#include <thread>
#include <vector>

namespace idt3 {

int hardwareJobs()
{
    unsigned n = std::thread::hardware_concurrency();
    return n == 0 ? 1 : static_cast<int>(n);
}

void parallelFor(int count, int jobs, const std::function<void(int, int)>& fn)
{
    if (count <= 0)
        return;
    if (jobs <= 0)
        jobs = hardwareJobs();
    jobs = std::min(jobs, count);
    if (jobs <= 1) {
        fn(0, count);
        return;
    }

    // Work is handed out in small chunks from a shared counter so uneven
    // per-item cost (e.g. complex vs. flat BC7 blocks) balances out.
    const int chunk = std::max(1, count / (jobs * 8));
    std::atomic<int> next(0);
    std::exception_ptr failure;
    std::mutex failureMutex;

    auto worker = [&]() {
        for (;;) {
            int begin = next.fetch_add(chunk);
            if (begin >= count)
                return;
            int end = std::min(count, begin + chunk);
            try {
                fn(begin, end);
            } catch (...) {
                std::lock_guard<std::mutex> lock(failureMutex);
                if (!failure)
                    failure = std::current_exception();
                // Drain the remaining work so all threads exit quickly.
                next.store(count);
                return;
            }
        }
    };

    std::vector<std::thread> threads;
    for (int i = 1; i < jobs; ++i)
        threads.push_back(std::thread(worker));
    worker();
    for (size_t i = 0; i < threads.size(); ++i)
        threads[i].join();
    if (failure)
        std::rethrow_exception(failure);
}

} // namespace idt3
