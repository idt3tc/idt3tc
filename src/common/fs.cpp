#include "common/fs.h"

#include "common/util.h"

#include <algorithm>
#include <cerrno>
#include <cstdio>
#include <cstring>

#ifdef _WIN32
#include <direct.h>
#include <sys/stat.h>
#include <windows.h>
#else
#include <dirent.h>
#include <sys/stat.h>
#include <sys/types.h>
#endif

namespace idt3 {
namespace fs {

namespace {

bool isSep(char c)
{
    return c == '/' || c == '\\';
}

// Index of the last path separator or npos.
size_t lastSep(const std::string& path)
{
    for (size_t i = path.size(); i > 0; --i) {
        if (isSep(path[i - 1]))
            return i - 1;
    }
    return std::string::npos;
}

void listInto(const std::string& dir, bool recursive, std::vector<std::string>& out)
{
#ifdef _WIN32
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(join(dir, "*").c_str(), &fd);
    if (h == INVALID_HANDLE_VALUE)
        throw Error("cannot list directory '" + dir + "'");
    do {
        std::string name = fd.cFileName;
        if (name == "." || name == "..")
            continue;
        std::string full = join(dir, name);
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
            if (recursive)
                listInto(full, recursive, out);
        } else {
            out.push_back(full);
        }
    } while (FindNextFileA(h, &fd));
    FindClose(h);
#else
    DIR* d = opendir(dir.c_str());
    if (!d)
        throw Error("cannot list directory '" + dir + "': " + std::strerror(errno));
    while (struct dirent* e = readdir(d)) {
        std::string name = e->d_name;
        if (name == "." || name == "..")
            continue;
        std::string full = join(dir, name);
        if (isDirectory(full)) {
            if (recursive)
                listInto(full, recursive, out);
        } else {
            out.push_back(full);
        }
    }
    closedir(d);
#endif
}

} // namespace

bool exists(const std::string& path)
{
    struct stat st;
    return stat(path.c_str(), &st) == 0;
}

bool isDirectory(const std::string& path)
{
    struct stat st;
    if (stat(path.c_str(), &st) != 0)
        return false;
#ifdef _WIN32
    return (st.st_mode & _S_IFDIR) != 0;
#else
    return S_ISDIR(st.st_mode);
#endif
}

void createDirectories(const std::string& path)
{
    if (path.empty() || isDirectory(path))
        return;
    std::string parent = dirname(path);
    if (!parent.empty() && parent != path)
        createDirectories(parent);
#ifdef _WIN32
    int rc = _mkdir(path.c_str());
#else
    int rc = mkdir(path.c_str(), 0777);
#endif
    if (rc != 0 && !isDirectory(path))
        throw Error("cannot create directory '" + path + "': " + std::strerror(errno));
}

std::vector<std::string> listFiles(const std::string& dir, bool recursive)
{
    std::vector<std::string> out;
    listInto(dir, recursive, out);
    std::sort(out.begin(), out.end());
    return out;
}

std::string dirname(const std::string& path)
{
    size_t p = lastSep(path);
    if (p == std::string::npos)
        return std::string();
    if (p == 0)
        return path.substr(0, 1); // root "/"
    return path.substr(0, p);
}

std::string basename(const std::string& path)
{
    size_t p = lastSep(path);
    return p == std::string::npos ? path : path.substr(p + 1);
}

std::string stem(const std::string& path)
{
    std::string base = basename(path);
    size_t dot = base.rfind('.');
    if (dot == std::string::npos || dot == 0)
        return base;
    return base.substr(0, dot);
}

std::string extension(const std::string& path)
{
    std::string base = basename(path);
    size_t dot = base.rfind('.');
    if (dot == std::string::npos || dot == 0)
        return std::string();
    return toLower(base.substr(dot + 1));
}

std::string withoutExtension(const std::string& path)
{
    std::string dir = dirname(path);
    std::string s = stem(path);
    if (dir.empty())
        return s;
    // Preserve the original separator between directory and file.
    return path.substr(0, lastSep(path) + 1) + s;
}

std::string join(const std::string& a, const std::string& b)
{
    if (a.empty())
        return b;
    if (isSep(a[a.size() - 1]))
        return a + b;
    return a + "/" + b;
}

std::string relativeTo(const std::string& path, const std::string& root)
{
    std::string r = root;
    while (r.size() > 1 && isSep(r[r.size() - 1]))
        r.erase(r.size() - 1);
    if (!r.empty() && path.size() > r.size() && path.compare(0, r.size(), r) == 0 && isSep(path[r.size()]))
        return path.substr(r.size() + 1);
    return basename(path);
}

std::vector<unsigned char> readFile(const std::string& path)
{
    FILE* f = std::fopen(path.c_str(), "rb");
    if (!f)
        throw Error("cannot open '" + path + "': " + std::strerror(errno));
    std::vector<unsigned char> data;
    unsigned char buf[65536];
    size_t n;
    while ((n = std::fread(buf, 1, sizeof(buf), f)) > 0)
        data.insert(data.end(), buf, buf + n);
    bool failed = std::ferror(f) != 0;
    std::fclose(f);
    if (failed)
        throw Error("error reading '" + path + "'");
    return data;
}

void writeFile(const std::string& path, const std::vector<unsigned char>& data)
{
    FILE* f = std::fopen(path.c_str(), "wb");
    if (!f)
        throw Error("cannot create '" + path + "': " + std::strerror(errno));
    size_t written = data.empty() ? 0 : std::fwrite(&data[0], 1, data.size(), f);
    bool failed = written != data.size();
    if (std::fclose(f) != 0)
        failed = true;
    if (failed) {
        std::remove(path.c_str());
        throw Error("error writing '" + path + "'");
    }
}

} // namespace fs
} // namespace idt3
