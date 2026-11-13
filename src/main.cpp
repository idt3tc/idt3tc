// idt3tc: tools for provisioning idTech 3 maps, mods and texture packs.

#include "common/util.h"
#include "common/version.h"
#include "img/cmd_img.h"

#include <cstdio>
#include <exception>
#include <string>
#include <vector>

namespace {

struct Command {
    const char* name;
    const char* summary;
    int (*run)(const std::vector<std::string>& args);
    void (*help)(const std::string& topic);
};

const Command kCommands[] = {
    {"img", "convert and process images (TGA, DDS/BCn, PNG, JPG) for idTech 3", idt3::img::runImgCommand,
     idt3::img::printImgHelp},
};

void printUsage()
{
    std::printf("usage: idt3tc [--version] [--help] COMMAND [ARGS...]\n\ncommands:\n");
    for (size_t i = 0; i < sizeof(kCommands) / sizeof(kCommands[0]); ++i)
        std::printf("  %-8s %s\n", kCommands[i].name, kCommands[i].summary);
    std::printf("\nRun 'idt3tc COMMAND --help' or 'idt3tc help COMMAND' for details.\n");
}

const Command* findCommand(const std::string& name)
{
    for (size_t i = 0; i < sizeof(kCommands) / sizeof(kCommands[0]); ++i) {
        if (name == kCommands[i].name)
            return &kCommands[i];
    }
    return nullptr;
}

int unknownCommand(const std::string& name)
{
    std::vector<std::string> names;
    for (size_t i = 0; i < sizeof(kCommands) / sizeof(kCommands[0]); ++i)
        names.push_back(kCommands[i].name);
    std::string hint = idt3::closestMatch(name, names);
    std::fprintf(stderr, "idt3tc: unknown command '%s'%s\n", name.c_str(),
                 hint.empty() ? "" : (" (did you mean '" + hint + "'?)").c_str());
    return 2;
}

} // namespace

int main(int argc, char** argv)
{
    std::vector<std::string> args(argv + 1, argv + argc);
    try {
        if (args.empty()) {
            printUsage();
            return 2;
        }
        const std::string& first = args[0];
        if (first == "-h" || first == "--help") {
            printUsage();
            return 0;
        }
        if (first == "--version" || first == "-V") {
            std::printf("idt3tc %s\n", IDT3_VERSION);
            return 0;
        }
        if (first == "help") {
            if (args.size() == 1) {
                printUsage();
                return 0;
            }
            const Command* c = findCommand(args[1]);
            if (!c)
                return unknownCommand(args[1]);
            c->help(args.size() > 2 ? args[2] : std::string());
            return 0;
        }
        const Command* c = findCommand(first);
        if (!c)
            return unknownCommand(first);
        return c->run(std::vector<std::string>(args.begin() + 1, args.end()));
    } catch (const idt3::UsageError& e) {
        std::fprintf(stderr, "idt3tc: %s\n", e.what());
        return 2;
    } catch (const std::exception& e) {
        std::fprintf(stderr, "idt3tc: %s\n", e.what());
        return 1;
    }
}
