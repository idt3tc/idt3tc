// Command line front end of `idt3tc img`: argument parsing, input expansion,
// pipeline execution and help output.

#include "img/cmd_img.h"

#include "common/fs.h"
#include "common/parallel.h"
#include "common/util.h"
#include "img/io.h"
#include "img/op.h"

#include <algorithm>
#include <cstdio>
#include <exception>
#include <memory>

namespace idt3 {
namespace img {

namespace {

const size_t kHelpWidth = 100;

struct Input {
    std::string path;
    std::string rel; // relative path below a directory input, without extension
};

struct PipelineStep {
    const OpInfo* info;
    std::unique_ptr<Op> op;
};

struct Settings {
    int jobs = 0; // 0 = all hardware threads
    bool recursive = false;
    bool keepGoing = false;
    int verbosity = 1;
    bool help = false;
    std::string helpTopic;
};

bool isOptionToken(const std::string& t)
{
    return t.size() > 1 && t[0] == '-';
}

// Global flags; returns true if `tok` (and possibly the next argument)
// was consumed.
bool parseGlobal(const std::vector<std::string>& args, size_t& i, Settings& s)
{
    const std::string& t = args[i];
    std::string value;
    bool hasValue = false;
    std::string name = t;
    size_t eq = t.find('=');
    if (startsWith(t, "--") && eq != std::string::npos) {
        name = t.substr(0, eq);
        value = t.substr(eq + 1);
        hasValue = true;
    }

    if (name == "-h" || name == "--help") {
        s.help = true;
        if (hasValue)
            s.helpTopic = value;
        else if (i + 1 < args.size() && !isOptionToken(args[i + 1]))
            s.helpTopic = args[++i];
        return true;
    }
    if (name == "-j" || name == "--jobs" || (startsWith(t, "-j") && t.size() > 2 && !startsWith(t, "--"))) {
        if (!hasValue) {
            if (t.size() > 2 && !startsWith(t, "--"))
                value = t.substr(2);
            else if (i + 1 < args.size())
                value = args[++i];
            else
                throw UsageError(name + ": missing number of jobs");
        }
        long n;
        if (!parseLong(value, n) || n < 0 || n > 1024)
            throw UsageError(name + ": expected a number of jobs 0..1024 (0 = all cores), got '" + value + "'");
        s.jobs = static_cast<int>(n);
        return true;
    }
    if (hasValue)
        return false;
    if (t == "-r" || t == "--recursive")
        s.recursive = true;
    else if (t == "-k" || t == "--keep-going")
        s.keepGoing = true;
    else if (t == "-q" || t == "--quiet")
        s.verbosity = 0;
    else if (t == "-v" || t == "--verbose")
        s.verbosity = 2;
    else
        return false;
    return true;
}

// Looks up an operation token ("--resize", "--resize=64x64", "-o").
const OpInfo* findOp(const std::string& tok, std::string& inlineArg, bool& hasInlineArg)
{
    hasInlineArg = false;
    std::string name;
    if (startsWith(tok, "--"))
        name = tok.substr(2);
    else if (tok.size() == 2 && tok[0] == '-')
        name = tok.substr(1);
    else
        return nullptr;
    size_t eq = name.find('=');
    if (eq != std::string::npos) {
        inlineArg = name.substr(eq + 1);
        name = name.substr(0, eq);
        hasInlineArg = true;
    }
    return OpRegistry::instance().find(name);
}

[[noreturn]] void unknownOp(const std::string& tok)
{
    std::string name = tok.substr(tok.find_first_not_of('-'));
    name = name.substr(0, name.find('='));
    std::string hint = closestMatch(name, OpRegistry::instance().names());
    throw UsageError("unknown operation or option '" + tok + "'" +
                     (hint.empty() ? std::string() : " (did you mean '--" + hint + "'?)") +
                     "; see 'idt3tc img --help'");
}

bool isKnownOptionKey(const OpInfo& info, const std::string& key)
{
    for (size_t i = 0; i < info.options.size(); ++i) {
        if (info.options[i].key == key)
            return true;
    }
    return false;
}

std::vector<std::string> optionKeys(const OpInfo& info)
{
    std::vector<std::string> k;
    for (size_t i = 0; i < info.options.size(); ++i)
        k.push_back(info.options[i].key);
    return k;
}

// Expands directory inputs to the image files they contain.
std::vector<Input> expandInputs(const std::vector<std::string>& raw, bool recursive)
{
    std::vector<Input> out;
    for (size_t i = 0; i < raw.size(); ++i) {
        const std::string& p = raw[i];
        if (fs::isDirectory(p)) {
            std::vector<std::string> files = fs::listFiles(p, recursive);
            size_t before = out.size();
            for (size_t f = 0; f < files.size(); ++f) {
                if (!isImageExtension(fs::extension(files[f])))
                    continue;
                Input in;
                in.path = files[f];
                in.rel = fs::withoutExtension(fs::relativeTo(files[f], p));
                out.push_back(in);
            }
            if (out.size() == before)
                throw UsageError("no image files found in directory '" + p + "'" +
                                 (recursive ? "" : " (use -r to include subdirectories)"));
        } else if (fs::exists(p)) {
            Input in;
            in.path = p;
            in.rel = fs::stem(p);
            out.push_back(in);
        } else {
            throw UsageError("input '" + p + "' does not exist");
        }
    }
    return out;
}

void printOpDetails(const OpInfo& info)
{
    std::string head = "--" + info.name + (info.arg.empty() ? "" : " " + info.arg);
    std::printf("%s\n", head.c_str());
    if (!info.aliases.empty()) {
        std::vector<std::string> a;
        for (size_t i = 0; i < info.aliases.size(); ++i)
            a.push_back((info.aliases[i].size() == 1 ? "-" : "--") + info.aliases[i]);
        std::printf("  aliases: %s\n", join(a, ", ").c_str());
    }
    std::printf("\n  %s\n", wrapText(info.summary, kHelpWidth, 2).c_str());
    if (!info.details.empty())
        std::printf("\n  %s\n", wrapText(info.details, kHelpWidth, 2).c_str());
    if (!info.options.empty()) {
        std::printf("\n  options (key=value, flags also as bare key):\n");
        size_t w = 0;
        for (size_t i = 0; i < info.options.size(); ++i)
            w = std::max(w, info.options[i].key.size() + info.options[i].def.size() + 3);
        w = std::min<size_t>(w, 30);
        for (size_t i = 0; i < info.options.size(); ++i) {
            const OptionHelp& o = info.options[i];
            std::string left = o.key + (o.def.empty() ? "" : "=" + o.def);
            std::string line = "    " + left + std::string(left.size() < w ? w - left.size() : 1, ' ') + "  ";
            std::string text = wrapText(o.help, kHelpWidth, line.size());
            if (left.size() >= w + 2) {
                // Long key/default: put the description on its own line.
                line = "    " + left + "\n" + std::string(w + 6, ' ');
                text = wrapText(o.help, kHelpWidth, w + 6);
            }
            std::printf("%s%s\n", line.c_str(), text.c_str());
        }
    }
}

} // namespace

void printImgHelp(const std::string& topic)
{
    const OpRegistry& reg = OpRegistry::instance();
    if (!topic.empty() && topic != "all") {
        std::string name = topic.substr(std::min(topic.find_first_not_of('-'), topic.size()));
        const OpInfo* info = reg.find(name);
        if (!info) {
            std::string hint = closestMatch(name, reg.names());
            throw UsageError("unknown operation '" + topic + "'" +
                             (hint.empty() ? std::string() : " (did you mean '" + hint + "'?)"));
        }
        printOpDetails(*info);
        return;
    }

    std::printf(
        "usage: idt3tc img [OPTIONS] [INPUT...] --OP [ARG] [key=value...] [--OP ...]\n"
        "\n"
        "Runs a pipeline of image operations, in the given order, once for every input. Inputs may be\n"
        "image files or directories (all images inside are processed). Without inputs, the pipeline has to\n"
        "create its image with --new or --load.\n"
        "\n"
        "options:\n"
        "  -j, --jobs N       worker threads (0 = all cores, default)\n"
        "  -r, --recursive    descend into subdirectories of directory inputs\n"
        "  -k, --keep-going   continue with the next input after an error\n"
        "  -q, --quiet        only print errors\n"
        "  -v, --verbose      print every step\n"
        "  -h, --help [OP]    this help, details of one operation, or 'all'\n");

    std::vector<std::string> categories;
    for (size_t i = 0; i < reg.all().size(); ++i) {
        if (std::find(categories.begin(), categories.end(), reg.all()[i].category) == categories.end())
            categories.push_back(reg.all()[i].category);
    }
    for (size_t c = 0; c < categories.size(); ++c) {
        std::printf("\n%s:\n", toLower(categories[c]).c_str());
        for (size_t i = 0; i < reg.all().size(); ++i) {
            const OpInfo& op = reg.all()[i];
            if (op.category != categories[c])
                continue;
            std::string left = "  --" + op.name + (op.arg.empty() ? "" : " " + op.arg);
            const size_t col = 30;
            if (left.size() + 2 > col)
                std::printf("%s\n%s%s\n", left.c_str(), std::string(col, ' ').c_str(),
                            wrapText(op.summary, kHelpWidth, col).c_str());
            else
                std::printf("%s%s%s\n", left.c_str(), std::string(col - left.size(), ' ').c_str(),
                            wrapText(op.summary, kHelpWidth, col).c_str());
        }
    }

    std::printf(
        "\n"
        "Every operation takes key=value options; see 'idt3tc img --help OP'.\n"
        "\n"
        "examples:\n"
        "  idt3tc img photo.jpg --pot --save textures/mymap/wall.tga\n"
        "  idt3tc img src/ -r --resize 50%% filter=lanczos3 --save out/{rel}.tga rle\n"
        "  idt3tc img grate.png --pot max=512 --save grate.dds codec=bc1a mip-coverage=0.5\n"
        "  idt3tc img height.png --normalmap strength=4 --save wall_n.dds codec=bc5\n"
        "  idt3tc img logo.png --rotate 15 --canvas 256x256 --pan 16,0 --save logo.tga\n");

    if (topic == "all") {
        for (size_t i = 0; i < reg.all().size(); ++i) {
            std::printf("\n");
            printOpDetails(reg.all()[i]);
        }
    }
}

int runImgCommand(const std::vector<std::string>& args)
{
    Settings settings;
    std::vector<std::string> rawInputs;
    std::vector<PipelineStep> steps;

    try {
        // Pass 1: split into globals, inputs and op groups.
        for (size_t i = 0; i < args.size(); ++i) {
            const std::string& t = args[i];
            if (t == "--") {
                // Tokens after "--" are inputs even if they start with '-',
                // up to the next operation.
                if (!steps.empty())
                    throw UsageError("'--' must come before the first operation");
                std::string ignored;
                bool ignoredFlag;
                for (++i; i < args.size() && !findOp(args[i], ignored, ignoredFlag); ++i)
                    rawInputs.push_back(args[i]);
                --i;
                continue;
            }
            if (isOptionToken(t) && parseGlobal(args, i, settings))
                continue;

            std::string inlineArg;
            bool hasInlineArg = false;
            const OpInfo* info = isOptionToken(t) ? findOp(t, inlineArg, hasInlineArg) : nullptr;
            if (!info) {
                if (isOptionToken(t))
                    unknownOp(t);
                if (!steps.empty()) {
                    throw UsageError("unexpected argument '" + t + "' after --" + steps.back().info->name +
                                     " (inputs must come before the first operation)");
                }
                rawInputs.push_back(t);
                continue;
            }

            const std::string ctxName = "--" + info->name;
            std::string arg;
            if (!info->arg.empty()) {
                if (hasInlineArg)
                    arg = inlineArg;
                else if (i + 1 < args.size())
                    arg = args[++i]; // may start with '-' (e.g. --rotate -90)
                else
                    throw UsageError(ctxName + ": missing argument " + info->arg);
            } else if (hasInlineArg) {
                throw UsageError(ctxName + " takes no argument");
            }

            // Options: key=value tokens or bare flag keys known to this op.
            std::vector<std::pair<std::string, std::string> > kv;
            while (i + 1 < args.size()) {
                const std::string& n = args[i + 1];
                if (isOptionToken(n))
                    break;
                size_t eq = n.find('=');
                if (eq != std::string::npos && eq > 0) {
                    kv.push_back(std::make_pair(n.substr(0, eq), n.substr(eq + 1)));
                } else if (isKnownOptionKey(*info, n)) {
                    kv.push_back(std::make_pair(n, std::string()));
                } else {
                    std::string hint = closestMatch(n, optionKeys(*info));
                    throw UsageError("unexpected argument '" + n + "' after " + ctxName +
                                     (hint.empty() ? std::string() : " (did you mean '" + hint + "'?)") +
                                     " (inputs must come before the first operation)");
                }
                ++i;
            }

            OptionReader reader(ctxName, kv);
            PipelineStep step;
            step.info = info;
            step.op = info->create(arg, reader);
            reader.finish();
            steps.push_back(std::move(step));
        }

        if (settings.help) {
            printImgHelp(settings.helpTopic);
            return 0;
        }
        if (steps.empty() && rawInputs.empty()) {
            printImgHelp(std::string());
            return 2;
        }
        if (steps.empty())
            throw UsageError("no operations given; e.g. add '--save {name}.tga' (see 'idt3tc img --help')");
    } catch (const UsageError& e) {
        std::fprintf(stderr, "idt3tc img: %s\n", e.what());
        return 2;
    }

    std::vector<Input> inputs;
    try {
        inputs = expandInputs(rawInputs, settings.recursive);

        // Writing several inputs to one fixed path silently overwrites
        // all but the last result.
        if (inputs.size() > 1) {
            for (size_t s = 0; s < steps.size(); ++s) {
                std::string tmpl = steps[s].op->outputTemplate();
                if (!tmpl.empty() && !hasInputPlaceholder(tmpl))
                    throw UsageError("--" + steps[s].info->name + " '" + tmpl + "' is the same for all " +
                                     strprintf("%zu", inputs.size()) +
                                     " inputs; use a placeholder such as {name} or {rel}");
            }
        }
    } catch (const UsageError& e) {
        std::fprintf(stderr, "idt3tc img: %s\n", e.what());
        return 2;
    }

    const int jobs = settings.jobs > 0 ? settings.jobs : hardwareJobs();
    const size_t runs = inputs.empty() ? 1 : inputs.size();
    size_t failed = 0;
    for (size_t r = 0; r < runs; ++r) {
        Context ctx;
        ctx.jobs = jobs;
        ctx.verbosity = settings.verbosity;
        ctx.index = static_cast<int>(r);
        ctx.count = static_cast<int>(runs);
        std::string label = inputs.empty() ? std::string("pipeline") : inputs[r].path;
        try {
            if (!inputs.empty()) {
                ctx.inputPath = inputs[r].path;
                ctx.inputRel = inputs[r].rel;
                LoadInfo li;
                ctx.image = loadImage(ctx.inputPath, &li);
                ctx.hasImage = true;
                ctx.verbose(strprintf("loaded %s (%dx%d, %s, %d channel%s, %d bit)", ctx.inputPath.c_str(),
                                      ctx.image.width, ctx.image.height, li.format.c_str(), li.channels,
                                      li.channels == 1 ? "" : "s", li.bitsPerChannel));
            }
            for (size_t s = 0; s < steps.size(); ++s) {
                ctx.verbose("  --" + steps[s].info->name);
                steps[s].op->apply(ctx);
            }
        } catch (const std::exception& e) {
            ++failed;
            std::fprintf(stderr, "idt3tc img: %s: %s\n", label.c_str(), e.what());
            if (!settings.keepGoing) {
                if (r + 1 < runs)
                    std::fprintf(stderr, "idt3tc img: stopping (use -k to continue with the remaining inputs)\n");
                return 1;
            }
        }
    }
    if (failed) {
        std::fprintf(stderr, "idt3tc img: %zu of %zu input%s failed\n", failed, runs, runs == 1 ? "" : "s");
        return 1;
    }
    return 0;
}

} // namespace img
} // namespace idt3
