// Test runner: runs every registered test, or those whose name contains
// the first command line argument.

#include "test.h"

#include <exception>

namespace idt3test {

std::vector<TestCase>& registry()
{
    static std::vector<TestCase> r;
    return r;
}

void fail(const char* file, int line, const std::string& msg)
{
    std::ostringstream os;
    os << file << ":" << line << ": " << msg;
    throw Failure{os.str()};
}

} // namespace idt3test

int main(int argc, char** argv)
{
    std::string filter = argc > 1 ? argv[1] : "";
    int run = 0, failed = 0;
    for (size_t i = 0; i < idt3test::registry().size(); ++i) {
        const idt3test::TestCase& t = idt3test::registry()[i];
        if (!filter.empty() && std::string(t.name).find(filter) == std::string::npos)
            continue;
        ++run;
        try {
            t.fn();
            std::printf("[ ok ] %s\n", t.name);
        } catch (const idt3test::Failure& f) {
            ++failed;
            std::printf("[FAIL] %s\n       %s\n", t.name, f.message.c_str());
        } catch (const std::exception& e) {
            ++failed;
            std::printf("[FAIL] %s\n       unexpected exception: %s\n", t.name, e.what());
        }
    }
    std::printf("%d of %d tests passed\n", run - failed, run);
    return failed || run == 0 ? 1 : 0;
}
