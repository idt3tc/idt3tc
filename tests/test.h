#pragma once

// Minimal self-registering test harness (no external framework).

#include <cmath>
#include <cstdio>
#include <sstream>
#include <string>
#include <vector>

namespace idt3test {

typedef void (*TestFn)();

struct TestCase {
    const char* name;
    TestFn fn;
};

std::vector<TestCase>& registry();

struct Registrar {
    Registrar(const char* name, TestFn fn) { registry().push_back(TestCase{name, fn}); }
};

// Thrown by failed checks; caught per test by the runner.
struct Failure {
    std::string message;
};

[[noreturn]] void fail(const char* file, int line, const std::string& msg);

} // namespace idt3test

#define IDT3_CAT2(a, b) a##b
#define IDT3_CAT(a, b) IDT3_CAT2(a, b)

#define TEST(name)                                                                                     \
    static void IDT3_CAT(test_, name)();                                                               \
    static idt3test::Registrar IDT3_CAT(registrar_, name)(#name, &IDT3_CAT(test_, name));              \
    static void IDT3_CAT(test_, name)()

#define CHECK(cond)                                                                                    \
    do {                                                                                               \
        if (!(cond))                                                                                   \
            idt3test::fail(__FILE__, __LINE__, "CHECK(" #cond ") failed");                             \
    } while (0)

#define CHECK_EQ(a, b)                                                                                 \
    do {                                                                                               \
        auto va_ = (a);                                                                                \
        auto vb_ = (b);                                                                                \
        if (!(va_ == vb_)) {                                                                           \
            std::ostringstream os_;                                                                    \
            os_ << "CHECK_EQ(" #a ", " #b ") failed: " << va_ << " != " << vb_;                        \
            idt3test::fail(__FILE__, __LINE__, os_.str());                                             \
        }                                                                                              \
    } while (0)

#define CHECK_NEAR(a, b, eps)                                                                          \
    do {                                                                                               \
        double va_ = (a), vb_ = (b);                                                                   \
        if (!(std::fabs(va_ - vb_) <= (eps))) {                                                        \
            std::ostringstream os_;                                                                    \
            os_ << "CHECK_NEAR(" #a ", " #b ") failed: " << va_ << " vs " << vb_ << " (eps " << (eps)  \
                << ")";                                                                                \
            idt3test::fail(__FILE__, __LINE__, os_.str());                                             \
        }                                                                                              \
    } while (0)

// Passes if `stmt` throws an exception of type `type`.
#define CHECK_THROWS(stmt, type)                                                                       \
    do {                                                                                               \
        bool thrown_ = false;                                                                          \
        try {                                                                                          \
            stmt;                                                                                      \
        } catch (const type&) {                                                                        \
            thrown_ = true;                                                                            \
        }                                                                                              \
        if (!thrown_)                                                                                  \
            idt3test::fail(__FILE__, __LINE__, "expected " #type " from: " #stmt);                     \
    } while (0)
