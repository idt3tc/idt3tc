#pragma once

#include "img/op.h"

#include <cmath>
#include <functional>
#include <string>

// Helpers shared by the op translation units.
namespace idt3 {
namespace img {

// An Op backed by a closure that captured the parsed parameters.
class FnOp : public Op {
public:
    FnOp(const std::function<void(Context&)>& fn, const std::string& outputTemplate)
        : fn_(fn), output_(outputTemplate)
    {
    }
    void apply(Context& ctx) const override { fn_(ctx); }
    std::string outputTemplate() const override { return output_; }

private:
    std::function<void(Context&)> fn_;
    std::string output_;
};

inline std::unique_ptr<Op> makeOp(const std::function<void(Context&)>& fn,
                                  const std::string& outputTemplate = std::string())
{
    return std::unique_ptr<Op>(new FnOp(fn, outputTemplate));
}

inline int roundToInt(double v)
{
    return static_cast<int>(std::floor(v + 0.5));
}

inline OptionHelp opt(const std::string& key, const std::string& def, const std::string& help)
{
    OptionHelp o;
    o.key = key;
    o.def = def;
    o.help = help;
    return o;
}

// Appends `extra` to `base` (used to combine shared option help blocks).
inline std::vector<OptionHelp> withOptions(std::vector<OptionHelp> base, const std::vector<OptionHelp>& extra)
{
    base.insert(base.end(), extra.begin(), extra.end());
    return base;
}

} // namespace img
} // namespace idt3
