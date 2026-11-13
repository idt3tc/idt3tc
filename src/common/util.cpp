#include "common/util.h"

#include <algorithm>
#include <cctype>
#include <cerrno>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <sstream>

namespace idt3 {

std::string strprintf(const char* fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    va_list ap2;
    va_copy(ap2, ap);
    int n = std::vsnprintf(nullptr, 0, fmt, ap);
    va_end(ap);
    std::string out;
    if (n > 0) {
        out.resize(static_cast<size_t>(n) + 1);
        std::vsnprintf(&out[0], out.size(), fmt, ap2);
        out.resize(static_cast<size_t>(n));
    }
    va_end(ap2);
    return out;
}

std::string toLower(std::string s)
{
    for (size_t i = 0; i < s.size(); ++i)
        s[i] = static_cast<char>(std::tolower(static_cast<unsigned char>(s[i])));
    return s;
}

bool startsWith(const std::string& s, const std::string& prefix)
{
    return s.size() >= prefix.size() && s.compare(0, prefix.size(), prefix) == 0;
}

bool endsWith(const std::string& s, const std::string& suffix)
{
    return s.size() >= suffix.size() && s.compare(s.size() - suffix.size(), suffix.size(), suffix) == 0;
}

std::vector<std::string> split(const std::string& s, char sep)
{
    std::vector<std::string> parts;
    std::string cur;
    for (size_t i = 0; i < s.size(); ++i) {
        if (s[i] == sep) {
            parts.push_back(cur);
            cur.clear();
        } else {
            cur += s[i];
        }
    }
    parts.push_back(cur);
    return parts;
}

std::string join(const std::vector<std::string>& parts, const std::string& sep)
{
    std::string out;
    for (size_t i = 0; i < parts.size(); ++i) {
        if (i)
            out += sep;
        out += parts[i];
    }
    return out;
}

bool parseDouble(const std::string& s, double& out)
{
    if (s.empty() || std::isspace(static_cast<unsigned char>(s[0])))
        return false;
    const char* begin = s.c_str();
    char* end = nullptr;
    errno = 0;
    double v = std::strtod(begin, &end);
    if (errno != 0 || end != begin + s.size())
        return false;
    // Reject nan/inf; they are never meaningful arguments here.
    if (!(v == v) || v > 1e300 || v < -1e300)
        return false;
    out = v;
    return true;
}

bool parseLong(const std::string& s, long& out)
{
    if (s.empty() || std::isspace(static_cast<unsigned char>(s[0])))
        return false;
    const char* begin = s.c_str();
    char* end = nullptr;
    errno = 0;
    long v = std::strtol(begin, &end, 10);
    if (errno != 0 || end != begin + s.size())
        return false;
    out = v;
    return true;
}

std::string formatNumber(double v)
{
    std::ostringstream ss;
    ss << v;
    return ss.str();
}

std::string wrapText(const std::string& text, size_t width, size_t indent)
{
    std::string out;
    // Explicit newlines in `text` are preserved as paragraph breaks.
    std::vector<std::string> paragraphs = split(text, '\n');
    for (size_t p = 0; p < paragraphs.size(); ++p) {
        if (p)
            out += "\n" + std::string(indent, ' ');
        std::istringstream words(paragraphs[p]);
        std::string word;
        size_t col = indent;
        bool first = true;
        while (words >> word) {
            if (!first && col + 1 + word.size() > width) {
                out += "\n" + std::string(indent, ' ');
                col = indent;
                first = true;
            }
            if (!first) {
                out += ' ';
                ++col;
            }
            out += word;
            col += word.size();
            first = false;
        }
    }
    return out;
}

size_t editDistance(const std::string& a, const std::string& b)
{
    std::vector<size_t> prev(b.size() + 1), cur(b.size() + 1);
    for (size_t j = 0; j <= b.size(); ++j)
        prev[j] = j;
    for (size_t i = 1; i <= a.size(); ++i) {
        cur[0] = i;
        for (size_t j = 1; j <= b.size(); ++j) {
            size_t sub = prev[j - 1] + (a[i - 1] == b[j - 1] ? 0 : 1);
            cur[j] = std::min(std::min(prev[j] + 1, cur[j - 1] + 1), sub);
        }
        prev.swap(cur);
    }
    return prev[b.size()];
}

std::string closestMatch(const std::string& word, const std::vector<std::string>& candidates)
{
    std::string best;
    size_t bestDist = static_cast<size_t>(-1);
    for (size_t i = 0; i < candidates.size(); ++i) {
        size_t d = editDistance(word, candidates[i]);
        if (d < bestDist) {
            bestDist = d;
            best = candidates[i];
        }
    }
    // Only suggest when the typo is small relative to the word length.
    if (bestDist <= std::max<size_t>(1, word.size() / 3))
        return best;
    return std::string();
}

} // namespace idt3
