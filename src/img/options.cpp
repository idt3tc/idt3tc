#include "img/options.h"

#include "common/util.h"

namespace idt3 {
namespace img {

OptionReader::OptionReader(const std::string& context,
                           const std::vector<std::pair<std::string, std::string> >& raw)
    : context_(context)
{
    for (size_t i = 0; i < raw.size(); ++i) {
        const std::string& key = raw[i].first;
        if (values_.count(key))
            throw UsageError(context_ + ": option '" + key + "' given more than once");
        values_[key] = raw[i].second;
        order_.push_back(key);
    }
}

bool OptionReader::has(const std::string& key) const
{
    return values_.count(key) != 0;
}

const std::string* OptionReader::lookup(const std::string& key)
{
    std::map<std::string, std::string>::const_iterator it = values_.find(key);
    if (it == values_.end())
        return nullptr;
    used_.insert(key);
    return &it->second;
}

std::string OptionReader::valueOf(const std::string& key)
{
    const std::string* v = lookup(key);
    if (v && v->empty())
        throw UsageError(context_ + ": option '" + key + "' needs a value (" + key + "=...)");
    return v ? *v : std::string();
}

void OptionReader::invalid(const std::string& key, const std::string& value, const std::string& expected) const
{
    throw UsageError(context_ + ": invalid value '" + value + "' for option '" + key + "' (expected " + expected +
                     ")");
}

std::string OptionReader::str(const std::string& key, const std::string& def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    return valueOf(key);
}

double OptionReader::number(const std::string& key, double def, double min, double max)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    double v;
    if (!parseDouble(s, v) || v < min || v > max)
        invalid(key, s, "a number in [" + formatNumber(min) + ", " + formatNumber(max) + "]");
    return v;
}

int OptionReader::integer(const std::string& key, int def, int min, int max)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    long v;
    if (!parseLong(s, v) || v < min || v > max)
        invalid(key, s, strprintf("an integer in [%d, %d]", min, max));
    return static_cast<int>(v);
}

bool OptionReader::flag(const std::string& key, bool def)
{
    const std::string* v = lookup(key);
    if (!v) {
        used_.insert(key);
        return def;
    }
    std::string s = toLower(*v);
    if (s.empty() || s == "1" || s == "true" || s == "yes" || s == "on")
        return true;
    if (s == "0" || s == "false" || s == "no" || s == "off")
        return false;
    invalid(key, *v, "1/0, true/false, yes/no or on/off");
}

Color OptionReader::color(const std::string& key, const Color& def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    Color c;
    if (!parseColor(s, c))
        invalid(key, s, "a color like #rrggbb[aa], r,g,b[,a] or a name");
    return c;
}

Filter OptionReader::filter(const std::string& key, Filter def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    Filter f;
    if (!parseFilter(s, f))
        invalid(key, s, "one of " + join(filterNames(), ", "));
    return f;
}

EdgeMode OptionReader::edge(const std::string& key, EdgeMode def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    EdgeMode m;
    if (!parseEdgeMode(s, m))
        invalid(key, s, "clamp, repeat, mirror or fill");
    return m;
}

Interp OptionReader::interp(const std::string& key, Interp def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    Interp m;
    if (!parseInterp(s, m))
        invalid(key, s, "nearest, bilinear or bicubic");
    return m;
}

Gravity OptionReader::gravity(const std::string& key, Gravity def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    Gravity g;
    if (!parseGravity(s, g))
        invalid(key, s, "nw, n, ne, w, center, e, sw, s or se");
    return g;
}

int OptionReader::channels(const std::string& key, int def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = valueOf(key);
    int mask;
    if (!parseChannels(s, mask))
        invalid(key, s, "a combination of r, g, b, a (e.g. rgb)");
    return mask;
}

int OptionReader::choice(const std::string& key, const std::vector<std::string>& choices, int def)
{
    if (!has(key)) {
        used_.insert(key);
        return def;
    }
    std::string s = toLower(valueOf(key));
    for (size_t i = 0; i < choices.size(); ++i) {
        if (s == choices[i])
            return static_cast<int>(i);
    }
    invalid(key, s, "one of " + join(choices, ", "));
}

ResampleOptions OptionReader::resampleOptions(Filter defFilter, EdgeMode defEdge, const std::string& prefix)
{
    ResampleOptions o;
    o.filter = filter(prefix + "filter", defFilter);
    o.edge = edge(prefix + "edge", defEdge);
    o.fill = color(prefix + "bg", Color(0, 0, 0, 0));
    o.linear = flag(prefix + "linear", false);
    o.premultiply = flag(prefix + "premultiply", true);
    o.blur = static_cast<float>(number(prefix + "blur", 1.0, 0.01, 100.0));
    return o;
}

void OptionReader::finish() const
{
    for (size_t i = 0; i < order_.size(); ++i) {
        if (used_.count(order_[i]))
            continue;
        // Every key the operation asked for is in used_, so it doubles as
        // the list of valid keys for a typo suggestion.
        std::string hint = closestMatch(order_[i], std::vector<std::string>(used_.begin(), used_.end()));
        throw UsageError(context_ + ": option '" + order_[i] + "' is not applicable here" +
                         (hint.empty() ? std::string() : " (did you mean '" + hint + "'?)"));
    }
}

} // namespace img
} // namespace idt3
