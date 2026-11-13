#pragma once

#include "img/context.h"
#include "img/options.h"

#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace idt3 {
namespace img {

// A parsed, validated pipeline step.
class Op {
public:
    virtual ~Op() {}
    virtual void apply(Context& ctx) const = 0;

    // Output path template for ops that write files (used to check that
    // batch runs do not write every input to the same file).
    virtual std::string outputTemplate() const { return std::string(); }
};

struct OptionHelp {
    std::string key;
    std::string def;  // default shown in help ("" = none)
    std::string help;
};

typedef std::function<std::unique_ptr<Op>(const std::string& arg, OptionReader& opts)> OpFactory;

// Static description of an operation: drives parsing and help output.
struct OpInfo {
    std::string name;                 // without leading dashes
    std::vector<std::string> aliases; // e.g. "o" for "-o"
    std::string arg;                  // placeholder of the required argument, "" if none
    std::string category;
    std::string summary;
    std::string details;
    std::vector<OptionHelp> options;
    OpFactory create;
};

class OpRegistry {
public:
    void add(const OpInfo& info);
    const OpInfo* find(const std::string& name) const;
    const std::vector<OpInfo>& all() const { return ops_; }
    std::vector<std::string> names() const;

    // The registry with every built-in operation.
    static const OpRegistry& instance();

private:
    std::vector<OpInfo> ops_;
};

// Registration hooks implemented by the op translation units.
void registerIoOps(OpRegistry& r);
void registerGeometryOps(OpRegistry& r);
void registerColorOps(OpRegistry& r);

// Help snippets for options shared by several operations.
std::vector<OptionHelp> resampleOptionHelp(const std::string& defFilter, const std::string& defEdge,
                                           const std::string& prefix = "");

} // namespace img
} // namespace idt3
