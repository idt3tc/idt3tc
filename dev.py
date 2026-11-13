#!/usr/bin/env python3
"""Build driver for idt3tc.

Cross compiles idt3tc with zig for every supported platform from any host pixi
runs on, collects the binaries under names rules_multitool can consume, and
checks that the build is reproducible:

    ./dev.py targets                    # list the platforms
    ./dev.py build                      # release build for the host platform
    ./dev.py build -a                   # ... for every platform
    ./dev.py build -t windows-x86_64    # ... for one platform (repeatable)
    ./dev.py test                       # build for the host and run the tests
    ./dev.py test --debug               # ... with zig's UBSan traps enabled
    ./dev.py dist                       # build every platform into ./dist
    ./dev.py dist --url-base URL        # ... plus a rules_multitool lock file
    ./dev.py repro                      # build twice, compare byte by byte
    ./dev.py version                    # print the project version
    ./dev.py clean

The commands that need the toolchain re-run themselves inside the pixi
environment when cmake, ninja or zig are not on PATH.
"""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BUILD_ROOT = ROOT / "build"
DIST_DIR = ROOT / "dist"
TOOLCHAIN = ROOT / "cmake" / "ZigToolchain.cmake"
TARGETS_FILE = ROOT / "cmake" / "targets.json"
DEPS_DOWNLOADS = ROOT / ".cache" / "deps" / "download"

# The tool name; every artifact is called `<NAME>-<platform>[.exe]`.
NAME = "idt3tc"

# The platform table is shared with cmake/ZigToolchain.cmake. Its keys are
# `<os>-<cpu>` spelled the way rules_multitool spells them, so the artifact
# names and the lock file entries are derived from the same place.
PLATFORMS = json.loads(TARGETS_FILE.read_text())["platforms"]

# Files `dist` writes next to the binaries.
CHECKSUMS = "SHA256SUMS"
MULTITOOL_LOCK = f"{NAME}.multitool.json"
MULTITOOL_SCHEMA = ("https://raw.githubusercontent.com/theoremlp/rules_multitool/"
                    "main/lockfile.schema.json")

# Set once we have re-executed ourselves inside the pixi environment, so that
# the second run never tries to do it again.
PIXI_GUARD = "IDT3_INSIDE_PIXI"


def required_tools():
    tools = ["cmake", "ninja", "zig"]
    # zig's archiver is broken on recent macOS; the toolchain file switches to
    # the LLVM tools pixi installs there.
    if platform.system() == "Darwin":
        tools += ["llvm-ar", "llvm-ranlib"]
    return tools


def reexec_in_pixi():
    """Re-runs this script inside `pixi run` when the toolchain is not on PATH.

    That way `./dev.py ...` works the same whether or not the caller already
    activated the environment.
    """
    tools = required_tools()
    if os.environ.get(PIXI_GUARD):
        missing = [tool for tool in tools if not shutil.which(tool)]
        if missing:
            sys.exit(f"dev.py: missing in the pixi environment: {', '.join(missing)}")
        return

    if all(shutil.which(tool) for tool in tools):
        return

    pixi = shutil.which("pixi")
    if pixi is None:
        sys.exit(f"dev.py: neither the required tools ({', '.join(tools)}) "
                 f"nor pixi were found on PATH")

    env = dict(os.environ, **{PIXI_GUARD: "1"})
    command = [pixi, "run", "--manifest-path", str(ROOT / "pixi.toml"),
               "python", str(ROOT / "dev.py"), *sys.argv[1:]]
    sys.exit(subprocess.call(command, env=env))


def run(command, **kwargs):
    text = [str(part) for part in command]
    print("+ " + " ".join(text), flush=True)
    subprocess.run(text, check=True, **kwargs)


def project_version():
    """The version from the project() call in CMakeLists.txt."""
    text = (ROOT / "CMakeLists.txt").read_text()
    match = re.search(r"project\([^)]*?\bVERSION\s+([0-9][0-9A-Za-z.\-]*)", text, re.S)
    if not match:
        sys.exit("dev.py: no VERSION found in the project() call of CMakeLists.txt")
    return match.group(1)


def host_platform():
    """The platform matching the machine this script is running on."""
    system = platform.system()
    machine = platform.machine().lower()
    cpu = "arm64" if machine in ("arm64", "aarch64") else "x86_64"
    os_name = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}.get(system)
    if os_name is None or f"{os_name}-{cpu}" not in PLATFORMS:
        sys.exit(f"dev.py: no platform for host {system}/{machine}")
    return f"{os_name}-{cpu}"


def executable_name(target):
    return NAME + (".exe" if PLATFORMS[target]["os"] == "windows" else "")


def artifact_name(target):
    """`idt3tc-linux-x86_64`, `idt3tc-windows-arm64.exe`, ..."""
    suffix = ".exe" if PLATFORMS[target]["os"] == "windows" else ""
    return f"{NAME}-{target}{suffix}"


def expected_dist_files():
    return sorted(artifact_name(target) for target in PLATFORMS)


def build_dir(target, config, tests):
    # Test builds get a directory of their own so that switching between
    # `build` and `test` does not reconfigure (and rebuild) either one.
    return BUILD_ROOT / f"{target}-{config.lower()}{'-tests' if tests else ''}"


def build_one(target, config="Release", tests=False, jobs=None, clean=False):
    """Configures, builds and installs one platform; returns the build dir."""
    output = build_dir(target, config, tests)
    stamp = output / ".idt3-build-inputs"
    inputs = json.dumps(PLATFORMS[target], sort_keys=True) + "\n"

    # The target triple is baked into the generated compiler shims rather than
    # into the compile command lines, so ninja cannot see it change and would
    # keep the old artifacts. Editing the platform entry therefore discards
    # the build directory.
    if stamp.is_file() and stamp.read_text() != inputs:
        print(f"build inputs changed, discarding {output}", flush=True)
        clean = True
    if clean:
        shutil.rmtree(output, ignore_errors=True)
    output.mkdir(parents=True, exist_ok=True)
    stamp.write_text(inputs)

    # CMake is cheap to re-run and skips the work when nothing changed, so it
    # is called unconditionally instead of checking for an existing cache.
    run(["cmake", "-S", ROOT, "-B", output, "-G", "Ninja",
         f"-DCMAKE_TOOLCHAIN_FILE={TOOLCHAIN}",
         f"-DIDT3_TARGET={target}",
         f"-DCMAKE_BUILD_TYPE={config}",
         f"-DIDT3_BUILD_TESTS={'ON' if tests else 'OFF'}",
         "-DIDT3_WARNINGS_AS_ERRORS=ON"])

    command = ["cmake", "--build", output]
    if jobs:
        command += ["--parallel", str(jobs)]
    run(command)
    run(["cmake", "--install", output, "--prefix", output / "install"])
    return output


def selected_targets(args):
    if args.all:
        return sorted(PLATFORMS)
    if args.target:
        unknown = [name for name in args.target if name not in PLATFORMS]
        if unknown:
            sys.exit(f"dev.py: unknown platform {', '.join(unknown)}; "
                     f"`./dev.py targets` lists them")
        return args.target
    return [host_platform()]


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_multitool_lock(dist_dir, url_base):
    """Writes the rules_multitool lock file entry for the collected binaries.

    The file is a complete lock file holding only this tool, so it can be
    used as is or merged into an existing multitool.lock.json.
    """
    binaries = []
    for target in sorted(PLATFORMS):
        name = artifact_name(target)
        binaries.append({
            "kind": "file",
            "url": f"{url_base.rstrip('/')}/{name}",
            "sha256": sha256(dist_dir / name),
            "os": PLATFORMS[target]["os"],
            "cpu": PLATFORMS[target]["cpu"],
        })
    lock = {"$schema": MULTITOOL_SCHEMA, NAME: {"binaries": binaries}}
    (dist_dir / MULTITOOL_LOCK).write_text(json.dumps(lock, indent=2) + "\n")


def collect_dist(dist_dir, url_base=None):
    """Copies every platform's release binary into `dist_dir` and checks it."""
    # Rebuilt from scratch every time, so that artifacts of a platform that was
    # dropped in the meantime cannot linger.
    shutil.rmtree(dist_dir, ignore_errors=True)
    dist_dir.mkdir(parents=True)

    for target in sorted(PLATFORMS):
        source = build_dir(target, "Release", False) / "install" / "bin" / executable_name(target)
        if not source.is_file():
            sys.exit(f"dev.py: {source} was not built")
        destination = dist_dir / artifact_name(target)
        shutil.copyfile(source, destination)
        # Mode bits are not part of the bytes compared, but a binary should
        # stay runnable straight out of ./dist.
        destination.chmod(0o755)

    names = expected_dist_files()
    found = sorted(path.name for path in dist_dir.iterdir())
    if found != names:
        sys.exit(f"dev.py: unexpected dist content {found}, expected {names}")

    # `sha256sum -c SHA256SUMS` compatible.
    (dist_dir / CHECKSUMS).write_text(
        "".join(f"{sha256(dist_dir / name)}  {name}\n" for name in names))
    if url_base:
        write_multitool_lock(dist_dir, url_base)

    for path in sorted(dist_dir.iterdir()):
        print(f"  {path.relative_to(ROOT) if ROOT in path.parents else path}"
              f"  {path.stat().st_size} B", flush=True)


def command_targets(_args):
    host = host_platform()
    width = max(len(name) for name in PLATFORMS)
    for name in sorted(PLATFORMS):
        note = "  (host)" if name == host else ""
        print(f"{name:<{width}}  {PLATFORMS[name]['triple']:<26}  "
              f"{artifact_name(name)}{note}")


def command_version(_args):
    print(project_version())


def command_build(args):
    for target in selected_targets(args):
        print(f"\n== {target} ==", flush=True)
        build_one(target, "Debug" if args.debug else "Release",
                  jobs=args.jobs, clean=args.clean)


def command_test(args):
    target = host_platform()
    output = build_one(target, "Debug" if args.debug else "Release", tests=True,
                       jobs=args.jobs, clean=args.clean)
    command = ["ctest", "--test-dir", output, "--output-on-failure"]
    if args.jobs:
        command += ["--parallel", str(args.jobs)]
    run(command)


def command_dist(args):
    for target in sorted(PLATFORMS):
        print(f"\n== {target} ==", flush=True)
        build_one(target, "Release", jobs=args.jobs, clean=args.clean)
    output = Path(args.output).resolve() if args.output else DIST_DIR
    print(f"\ncollecting into {output}", flush=True)
    collect_dist(output, args.url_base)


def source_files():
    """The files a fresh checkout of the current working tree consists of."""
    listing = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT, check=True, capture_output=True).stdout.decode()
    # --cached still lists files deleted from the working tree.
    return [name for name in listing.split("\0") if name and (ROOT / name).is_file()]


def command_repro(args):
    """Builds the dist twice, as differently as is practical, and compares.

    The first build is the regular one in this checkout. The second runs in a
    copy of the working tree at another path, with its own deps extraction and
    an empty zig cache, so that zig's runtime libraries (libc++, compiler-rt,
    musl, mingw) are compiled afresh too. Only the downloaded dependency
    archives are shared, which are hash-pinned anyway.
    """
    command_dist(argparse.Namespace(jobs=args.jobs, clean=args.clean,
                                    output=None, url_base=None))

    scratch = Path(tempfile.mkdtemp(prefix="idt3-repro-"))
    copy = scratch / "checkout"
    for name in source_files():
        (copy / name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, copy / name)
    if DEPS_DOWNLOADS.is_dir():
        shutil.copytree(DEPS_DOWNLOADS, copy / ".cache" / "deps" / "download")

    env = dict(os.environ, **{
        PIXI_GUARD: "1",
        "ZIG_GLOBAL_CACHE_DIR": str(scratch / "zig-global-cache"),
        "ZIG_LOCAL_CACHE_DIR": str(scratch / "zig-local-cache"),
    })
    command = [sys.executable, copy / "dev.py", "dist"]
    if args.jobs:
        command += ["--jobs", str(args.jobs)]
    print(f"\n== second build in {copy} ==", flush=True)
    run(command, env=env)

    first, second = DIST_DIR, copy / "dist"
    names = sorted(path.name for path in first.iterdir())
    if names != sorted(path.name for path in second.iterdir()):
        sys.exit(f"dev.py: the two builds produced different file sets "
                 f"({first} vs {second})")

    differing = [name for name in names if sha256(first / name) != sha256(second / name)]
    print()
    for name in names:
        state = "DIFFERS" if name in differing else "identical"
        print(f"  {state:<9}  {sha256(first / name)}  {name}")

    if differing:
        sys.exit(f"\ndev.py: not reproducible: {', '.join(differing)}; second build "
                 f"kept in {copy} (diffoscope compares them in detail)")

    if args.keep:
        print(f"\nsecond build kept in {copy}")
    else:
        shutil.rmtree(scratch, ignore_errors=True)
    print(f"\nreproducible: {len(names)} files identical")


def command_clean(_args):
    for directory in (BUILD_ROOT, DIST_DIR):
        print(f"removing {directory}", flush=True)
        shutil.rmtree(directory, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subparsers = parser.add_subparsers(dest="command", required=True)

    def add_common(subparser, debug=True):
        subparser.add_argument("-j", "--jobs", type=int, help="parallel build jobs")
        subparser.add_argument("--clean", action="store_true",
                               help="discard the existing build directories first")
        if debug:
            subparser.add_argument("--debug", action="store_true",
                                   help="debug build (zig traps on undefined behaviour)")

    targets = subparsers.add_parser("targets", help="list the supported platforms")
    targets.set_defaults(func=command_targets, toolchain=False)

    version = subparsers.add_parser("version", help="print the project version")
    version.set_defaults(func=command_version, toolchain=False)

    build = subparsers.add_parser("build", help="cross compile for some platforms")
    add_common(build)
    build.add_argument("-a", "--all", action="store_true", help="build every platform")
    build.add_argument("-t", "--target", action="append", metavar="PLATFORM",
                       help="platform to build (repeatable, default: the host)")
    build.set_defaults(func=command_build, toolchain=True)

    test = subparsers.add_parser("test", help="build for the host and run the tests")
    add_common(test)
    test.set_defaults(func=command_test, toolchain=True)

    dist = subparsers.add_parser(
        "dist", help="release build of every platform, collected into ./dist")
    add_common(dist, debug=False)
    dist.add_argument("-o", "--output", metavar="DIR",
                      help="collect into DIR instead of ./dist")
    dist.add_argument("--url-base", metavar="URL",
                      help=f"also write {MULTITOOL_LOCK} for binaries published "
                           f"under URL (e.g. a GitHub release download URL)")
    dist.set_defaults(func=command_dist, toolchain=True)

    repro = subparsers.add_parser(
        "repro", help="build the dist twice independently and compare the bytes")
    add_common(repro, debug=False)
    repro.add_argument("--keep", action="store_true",
                       help="keep the second build even when it is identical")
    repro.set_defaults(func=command_repro, toolchain=True)

    clean = subparsers.add_parser("clean", help="remove ./build and ./dist")
    clean.set_defaults(func=command_clean, toolchain=False)

    args = parser.parse_args()
    if args.toolchain:
        reexec_in_pixi()

    try:
        args.func(args)
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)


if __name__ == "__main__":
    main()
