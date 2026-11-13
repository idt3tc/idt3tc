#pragma once

#include <string>
#include <vector>

// Minimal filesystem helpers; C++11 has no <filesystem>.
namespace idt3 {
namespace fs {

bool exists(const std::string& path);
bool isDirectory(const std::string& path);

// Creates `path` and all missing parents. Throws idt3::Error on failure.
void createDirectories(const std::string& path);

// Lists regular files below `dir` (sorted, paths include `dir`).
std::vector<std::string> listFiles(const std::string& dir, bool recursive);

// Path decomposition; both '/' and '\\' are accepted as separators.
std::string dirname(const std::string& path);   // "a/b/c.png" -> "a/b", "c.png" -> ""
std::string basename(const std::string& path);  // "a/b/c.png" -> "c.png"
std::string stem(const std::string& path);      // "a/b/c.png" -> "c"
std::string extension(const std::string& path); // "a/b/c.PNG" -> "png" (lowercase, no dot)
std::string withoutExtension(const std::string& path); // "a/b/c.png" -> "a/b/c"
std::string join(const std::string& a, const std::string& b);

// Path of `path` relative to directory `root` if `path` is located below
// it, otherwise just the basename.
std::string relativeTo(const std::string& path, const std::string& root);

std::vector<unsigned char> readFile(const std::string& path);
void writeFile(const std::string& path, const std::vector<unsigned char>& data);

} // namespace fs
} // namespace idt3
