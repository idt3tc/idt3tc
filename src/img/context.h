#pragma once

#include "img/image.h"

#include <string>

namespace idt3 {
namespace img {

// State threaded through one pipeline run (one run per input file).
struct Context {
    Image image;
    bool hasImage;

    // The input this run was started for; empty when the pipeline builds
    // its image itself (--new / --load).
    std::string inputPath;
    // Input path relative to the directory it was found in when a directory
    // was given as input (without extension), else the input stem.
    std::string inputRel;
    int index; // 0-based input index
    int count; // number of inputs

    int jobs;
    int verbosity; // 0 = quiet, 1 = normal, 2 = verbose

    Context() : hasImage(false), index(0), count(1), jobs(1), verbosity(1) {}

    // Throws if no image has been loaded yet.
    Image& requireImage(const std::string& op);

    // Expands {name}, {stem}, {ext}, {dir}, {rel}, {index}, {w}, {h} and
    // {{ / }} in an output path template.
    std::string expandPath(const std::string& tmpl) const;

    void info(const std::string& msg) const;    // shown unless quiet
    void verbose(const std::string& msg) const; // shown with -v
};

// True if `tmpl` contains any placeholder that differs between inputs.
bool hasInputPlaceholder(const std::string& tmpl);

} // namespace img
} // namespace idt3
