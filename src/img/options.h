#pragma once

#include "img/image.h"
#include "img/resample.h"
#include "img/sampling.h"
#include "img/spec.h"

#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

namespace idt3 {
namespace img {

// key=value modifiers given after an operation, e.g.
//   --resize 512x512 filter=lanczos3 linear
// A bare key (no '=') is shorthand for key=1 and only valid for flags.
//
// Every accessor marks its key as used; finish() rejects keys that the
// operation never asked for, so typos and options that do not apply in the
// chosen mode (e.g. DDS options on a TGA save) are reported instead of
// being silently ignored.
class OptionReader {
public:
    OptionReader(const std::string& context, const std::vector<std::pair<std::string, std::string> >& raw);

    bool has(const std::string& key) const;

    std::string str(const std::string& key, const std::string& def);
    double number(const std::string& key, double def, double min, double max);
    int integer(const std::string& key, int def, int min, int max);
    bool flag(const std::string& key, bool def);
    Color color(const std::string& key, const Color& def);
    Filter filter(const std::string& key, Filter def);
    EdgeMode edge(const std::string& key, EdgeMode def);
    Interp interp(const std::string& key, Interp def);
    Gravity gravity(const std::string& key, Gravity def);
    int channels(const std::string& key, int def);

    // One of a fixed set of words; returns the index into `choices`.
    int choice(const std::string& key, const std::vector<std::string>& choices, int def);

    // Common resampling knobs: filter, edge, bg, linear, premultiply, blur.
    ResampleOptions resampleOptions(Filter defFilter, EdgeMode defEdge, const std::string& prefix = "");

    // Throws UsageError for unused keys.
    void finish() const;

    const std::string& context() const { return context_; }

private:
    const std::string* lookup(const std::string& key);
    std::string valueOf(const std::string& key);
    [[noreturn]] void invalid(const std::string& key, const std::string& value, const std::string& expected) const;

    std::string context_;
    std::map<std::string, std::string> values_;
    std::set<std::string> bare_;
    std::set<std::string> used_;
    std::vector<std::string> order_;
};

} // namespace img
} // namespace idt3
