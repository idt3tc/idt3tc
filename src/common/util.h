#pragma once

#include <memory>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace idt3 {

// C++11 lacks std::make_unique.
template <class T, class... Args>
std::unique_ptr<T> make_unique(Args&&... args)
{
    return std::unique_ptr<T>(new T(std::forward<Args>(args)...));
}

// Runtime failure while processing (I/O, decoding, invalid image state).
class Error : public std::runtime_error {
public:
    explicit Error(const std::string& msg) : std::runtime_error(msg) {}
};

// Invalid command line; reported with exit code 2.
class UsageError : public Error {
public:
    explicit UsageError(const std::string& msg) : Error(msg) {}
};

// printf-style formatting into a std::string.
std::string strprintf(const char* fmt, ...)
#if defined(__GNUC__)
    __attribute__((format(printf, 1, 2)))
#endif
    ;

std::string toLower(std::string s);
bool startsWith(const std::string& s, const std::string& prefix);
bool endsWith(const std::string& s, const std::string& suffix);
std::vector<std::string> split(const std::string& s, char sep);
std::string join(const std::vector<std::string>& parts, const std::string& sep);

// Strict number parsing: the whole string must be consumed.
bool parseDouble(const std::string& s, double& out);
bool parseLong(const std::string& s, long& out);

// Formats a float compactly ("0.5", "2", "1e-05").
std::string formatNumber(double v);

// Word wrapping for help output; continuation lines get `indent` spaces.
std::string wrapText(const std::string& text, size_t width, size_t indent);

// Levenshtein distance, used for "did you mean" suggestions.
size_t editDistance(const std::string& a, const std::string& b);

// Returns the candidate closest to `word`, or an empty string when nothing
// is reasonably close.
std::string closestMatch(const std::string& word, const std::vector<std::string>& candidates);

} // namespace idt3
