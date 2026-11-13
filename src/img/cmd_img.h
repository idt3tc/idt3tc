#pragma once

#include <string>
#include <vector>

namespace idt3 {
namespace img {

// Entry point of `idt3tc img`; `args` excludes the program and subcommand
// names. Returns the process exit code: 0 success, 1 processing error,
// 2 usage error.
int runImgCommand(const std::vector<std::string>& args);

// Prints the `idt3tc img` usage overview (or details for one operation).
void printImgHelp(const std::string& op);

} // namespace img
} // namespace idt3
