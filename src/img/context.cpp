#include "img/context.h"

#include "common/fs.h"
#include "common/util.h"

#include <cstdio>

namespace idt3 {
namespace img {

Image& Context::requireImage(const std::string& op)
{
    if (!hasImage)
        throw UsageError(op + ": no image loaded (give an input file, --load FILE or --new WxH first)");
    return image;
}

std::string Context::expandPath(const std::string& tmpl) const
{
    std::string out;
    size_t i = 0;
    while (i < tmpl.size()) {
        if (tmpl.compare(i, 2, "{{") == 0) {
            out += '{';
            i += 2;
            continue;
        }
        if (tmpl.compare(i, 2, "}}") == 0) {
            out += '}';
            i += 2;
            continue;
        }
        if (tmpl[i] != '{') {
            out += tmpl[i++];
            continue;
        }
        size_t close = tmpl.find('}', i);
        if (close == std::string::npos)
            throw UsageError("unterminated '{' in output path '" + tmpl + "'");
        std::string key = tmpl.substr(i + 1, close - i - 1);
        if (key == "name" || key == "stem")
            out += inputPath.empty() ? std::string("image") : fs::stem(inputPath);
        else if (key == "ext")
            out += fs::extension(inputPath);
        else if (key == "dir") {
            std::string d = fs::dirname(inputPath);
            out += d.empty() ? std::string(".") : d;
        } else if (key == "rel")
            out += inputRel.empty() ? std::string("image") : inputRel;
        else if (key == "index")
            out += strprintf("%d", index);
        else if (key == "w")
            out += strprintf("%d", image.width);
        else if (key == "h")
            out += strprintf("%d", image.height);
        else
            throw UsageError("unknown placeholder '{" + key +
                             "}' in output path (known: name, ext, dir, rel, index, w, h)");
        i = close + 1;
    }
    return out;
}

void Context::info(const std::string& msg) const
{
    if (verbosity >= 1)
        std::fprintf(stderr, "%s\n", msg.c_str());
}

void Context::verbose(const std::string& msg) const
{
    if (verbosity >= 2)
        std::fprintf(stderr, "%s\n", msg.c_str());
}

bool hasInputPlaceholder(const std::string& tmpl)
{
    const char* keys[] = {"{name}", "{stem}", "{ext}", "{dir}", "{rel}", "{index}"};
    for (size_t i = 0; i < sizeof(keys) / sizeof(keys[0]); ++i) {
        if (tmpl.find(keys[i]) != std::string::npos)
            return true;
    }
    return false;
}

} // namespace img
} // namespace idt3
