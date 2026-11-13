#include "img/op.h"

#include "common/util.h"

namespace idt3 {
namespace img {

void OpRegistry::add(const OpInfo& info)
{
    if (find(info.name))
        throw Error("duplicate operation '" + info.name + "'");
    ops_.push_back(info);
}

const OpInfo* OpRegistry::find(const std::string& name) const
{
    for (size_t i = 0; i < ops_.size(); ++i) {
        if (ops_[i].name == name)
            return &ops_[i];
        for (size_t a = 0; a < ops_[i].aliases.size(); ++a) {
            if (ops_[i].aliases[a] == name)
                return &ops_[i];
        }
    }
    return nullptr;
}

std::vector<std::string> OpRegistry::names() const
{
    std::vector<std::string> n;
    for (size_t i = 0; i < ops_.size(); ++i)
        n.push_back(ops_[i].name);
    return n;
}

const OpRegistry& OpRegistry::instance()
{
    // Built on first use; function-local statics are thread-safe in C++11.
    static const OpRegistry registry = []() {
        OpRegistry r;
        registerIoOps(r);
        registerGeometryOps(r);
        registerColorOps(r);
        return r;
    }();
    return registry;
}

std::vector<OptionHelp> resampleOptionHelp(const std::string& defFilter, const std::string& defEdge,
                                           const std::string& prefix)
{
    std::vector<OptionHelp> o;
    o.push_back(OptionHelp{prefix + "filter", defFilter, "resampling filter: " + join(filterNames(), ", ")});
    o.push_back(OptionHelp{prefix + "edge", defEdge,
                           "how the filter samples beyond the border: clamp, repeat (tiling textures), mirror, fill"});
    o.push_back(OptionHelp{prefix + "bg", "transparent", "color used with edge=fill"});
    o.push_back(OptionHelp{prefix + "linear", "0", "filter in linear light instead of sRGB (gamma-correct)"});
    o.push_back(OptionHelp{prefix + "premultiply", "1",
                           "weight colors by alpha while filtering (prevents dark fringes around cutouts)"});
    o.push_back(OptionHelp{prefix + "blur", "1", "filter width multiplier; >1 softer, <1 sharper"});
    return o;
}

} // namespace img
} // namespace idt3
