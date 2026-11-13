#!/usr/bin/env python3
"""Build driver for idt3tc.

Cross compiles idt3tc for every supported platform from any host pixi runs
on: cargo-zigbuild (zig as C++ compiler and linker) for Linux and macOS,
cargo-xwin (clang-cl, lld-link and the MSVC CRT/Windows SDK downloaded by
xwin) for Windows. It collects the binaries under names rules_multitool can
consume and checks that the build is reproducible:

    ./dev.py targets                    # list the platforms
    ./dev.py build                      # release build for the host platform
    ./dev.py build -a                   # ... for every platform
    ./dev.py build -t windows-x86_64    # ... for one platform (repeatable)
    ./dev.py test                       # run the tests on the host
    ./dev.py test --release             # ... with the release profile
    ./dev.py dist                       # build every platform into ./dist
    ./dev.py dist --url-base URL        # ... plus a rules_multitool lock file
    ./dev.py repro                      # build twice, compare byte by byte
    ./dev.py version                    # print the project version
    ./dev.py clean

The commands that need the toolchain re-run themselves inside the pixi
environment when zig, cargo-zigbuild or the LLVM tools are not on PATH. Rust
itself comes from rustup, pinned by rust-toolchain.toml.
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
TARGET_DIR = ROOT / "target"
DIST_DIR = ROOT / "dist"
CACHE_DIR = ROOT / ".cache"
# cargo-xwin is not packaged on conda-forge; dev.py installs this exact
# version here with `cargo install --locked`.
TOOLS_DIR = CACHE_DIR / "tools"
CARGO_XWIN_VERSION = "0.23.1"

# The MSVC CRT and Windows SDK that xwin downloads end up in the Windows
# binaries (static CRT), so they are pinned like every other toolchain part.
XWIN_VERSIONS = {
    "XWIN_VERSION": "17",
    "XWIN_CRT_VERSION": "14.44.17.14",
    "XWIN_SDK_VERSION": "10.0.26100",
}
XWIN_CACHE = CACHE_DIR / "xwin" / (XWIN_VERSIONS["XWIN_CRT_VERSION"] + "-" +
                                   XWIN_VERSIONS["XWIN_SDK_VERSION"])

# The tool name; every artifact is called `<NAME>-<platform>[.exe]`.
NAME = "idt3tc"

# Keys are `<os>-<cpu>` spelled the way rules_multitool spells them, so the
# artifact names and the lock file entries are derived from the same place.
# `builder` is the cargo subcommand that cross compiles the target; `macos`
# is the minimum macOS version (MACOSX_DEPLOYMENT_TARGET).
PLATFORMS = {
    "linux-x86_64": {"os": "linux", "cpu": "x86_64",
                     "triple": "x86_64-unknown-linux-musl", "builder": "zigbuild"},
    "linux-arm64": {"os": "linux", "cpu": "arm64",
                    "triple": "aarch64-unknown-linux-musl", "builder": "zigbuild"},
    "macos-x86_64": {"os": "macos", "cpu": "x86_64", "macos": "10.13",
                     "triple": "x86_64-apple-darwin", "builder": "zigbuild"},
    "macos-arm64": {"os": "macos", "cpu": "arm64", "macos": "11.0",
                    "triple": "aarch64-apple-darwin", "builder": "zigbuild"},
    "windows-x86_64": {"os": "windows", "cpu": "x86_64",
                       "triple": "x86_64-pc-windows-msvc", "builder": "xwin"},
    "windows-arm64": {"os": "windows", "cpu": "arm64",
                      "triple": "aarch64-pc-windows-msvc", "builder": "xwin"},
}

# Files `dist` writes next to the binaries.
CHECKSUMS = "SHA256SUMS"
MULTITOOL_LOCK = f"{NAME}.multitool.json"
MULTITOOL_SCHEMA = ("https://raw.githubusercontent.com/theoremlp/rules_multitool/"
                    "main/lockfile.schema.json")

# Set once we have re-executed ourselves inside the pixi environment, so that
# the second run never tries to do it again.
PIXI_GUARD = "IDT3_INSIDE_PIXI"

# Provided by the pixi environment.
PIXI_TOOLS = ["zig", "cargo-zigbuild", "clang-cl", "lld-link", "llvm-lib"]


def reexec_in_pixi():
    """Re-runs this script inside `pixi run` when the toolchain is not on PATH.

    That way `./dev.py ...` works the same whether or not the caller already
    activated the environment.
    """
    if shutil.which("cargo") is None or shutil.which("rustup") is None:
        sys.exit("dev.py: cargo/rustup not found on PATH; install rustup "
                 "(https://rustup.rs), rust-toolchain.toml selects the version")

    if os.environ.get(PIXI_GUARD):
        missing = [tool for tool in PIXI_TOOLS if not shutil.which(tool)]
        if missing:
            sys.exit(f"dev.py: missing in the pixi environment: {', '.join(missing)}")
        return

    if all(shutil.which(tool) for tool in PIXI_TOOLS):
        return

    pixi = shutil.which("pixi")
    if pixi is None:
        sys.exit(f"dev.py: neither the required tools ({', '.join(PIXI_TOOLS)}) "
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
    """The version from the [package] table of Cargo.toml."""
    text = (ROOT / "Cargo.toml").read_text()
    match = re.search(r'^\[package\][^\[]*?^version\s*=\s*"([^"]+)"', text, re.S | re.M)
    if not match:
        sys.exit("dev.py: no version found in the [package] table of Cargo.toml")
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
    return f"{NAME}-{target}" + (".exe" if PLATFORMS[target]["os"] == "windows" else "")


def expected_dist_files():
    return sorted(artifact_name(target) for target in PLATFORMS)


def binary_path(target, profile="release"):
    return TARGET_DIR / PLATFORMS[target]["triple"] / profile / executable_name(target)


def cargo_home():
    return Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo").resolve()


def ensure_cargo_xwin():
    """Installs the pinned cargo-xwin into TOOLS_DIR unless it is there."""
    binary = TOOLS_DIR / "bin" / ("cargo-xwin" + (".exe" if os.name == "nt" else ""))
    if binary.is_file():
        installed = subprocess.run([binary, "xwin", "--version"], capture_output=True,
                                   text=True).stdout.split()
        if installed[-1:] == [CARGO_XWIN_VERSION]:
            return
    run(["cargo", "install", "--locked", "--quiet", "--force", "--root", TOOLS_DIR,
         f"cargo-xwin@{CARGO_XWIN_VERSION}"], cwd=ROOT)


def build_env():
    """Environment shared by every cargo invocation."""
    env = dict(os.environ)
    env["PATH"] = str(TOOLS_DIR / "bin") + os.pathsep + env.get("PATH", "")
    env["XWIN_CACHE_DIR"] = str(XWIN_CACHE)
    # Intermediate artifacts stay in ./target too, whatever build-dir a
    # user's cargo config sets: --clean must really start from scratch, and
    # objects built by some other toolchain (say, a system zig) must not
    # end up in the binaries.
    env["CARGO_BUILD_BUILD_DIR"] = str(TARGET_DIR)
    env.update(XWIN_VERSIONS)
    return env


def rustflags_config(target):
    """`--config` arguments adding to the target's rustflags.

    Cargo merges these arrays with the ones in .cargo/config.toml (the static
    CRT of the MSVC targets), whereas RUSTFLAGS would replace them. The path
    remapping keeps the checkout and cargo home locations out of panic
    messages, so the binaries do not depend on where they were built.
    """
    flags = [f"--remap-path-prefix={cargo_home()}=/cargo",
             f"--remap-path-prefix={ROOT}=/src"]
    triple = PLATFORMS[target]["triple"]
    return ["--config", f"target.{triple}.rustflags={json.dumps(flags)}"]


def build_one(target, debug=False, jobs=None):
    """Cross compiles one platform; returns the path of the binary."""
    info = PLATFORMS[target]
    env = build_env()
    if info["builder"] == "xwin":
        ensure_cargo_xwin()
        command = ["cargo", "xwin", "build"]
    else:
        command = ["cargo", "zigbuild"]
    if "macos" in info:
        env["MACOSX_DEPLOYMENT_TARGET"] = info["macos"]
    command += ["--locked", "--target", info["triple"], "--target-dir", TARGET_DIR,
                *rustflags_config(target)]
    if not debug:
        command.append("--release")
    if jobs:
        command += ["--jobs", str(jobs)]
    # For Apple targets, the compiler version probe (`zig c++ --version`
    # plus the -L arguments cargo-zigbuild adds) leaves an empty a.o in the
    # build script's working directory, the checkout root. Only a file the
    # build itself created is removed.
    stray = ROOT / "a.o"
    stray_existed = stray.exists()
    run(command, cwd=ROOT, env=env)
    if not stray_existed and stray.is_file() and stray.stat().st_size == 0:
        stray.unlink()
    binary = binary_path(target, "debug" if debug else "release")
    if not binary.is_file():
        sys.exit(f"dev.py: {binary} was not built")
    return binary


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
        source = binary_path(target)
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
        if args.clean:
            shutil.rmtree(TARGET_DIR / PLATFORMS[target]["triple"], ignore_errors=True)
        print(f"built {build_one(target, debug=args.debug, jobs=args.jobs)}", flush=True)


def command_test(args):
    """Runs the test suites natively on the host.

    The default test profile keeps debug assertions and integer overflow
    checks enabled, so arithmetic bugs on crafted input fail loudly;
    --release runs the same tests with the profile the binaries ship with.
    """
    if args.clean:
        shutil.rmtree(TARGET_DIR, ignore_errors=True)
    command = ["cargo", "test", "--locked", "--target-dir", TARGET_DIR]
    if args.release:
        command.append("--release")
    if args.jobs:
        command += ["--jobs", str(args.jobs)]
    run(command, cwd=ROOT, env=build_env())


def command_dist(args):
    if args.clean:
        shutil.rmtree(TARGET_DIR, ignore_errors=True)
    for target in sorted(PLATFORMS):
        print(f"\n== {target} ==", flush=True)
        build_one(target, jobs=args.jobs)
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
    copy of the working tree at another path, with its own cargo target
    directory and an empty zig cache, so that zig's runtime libraries
    (libc++, compiler-rt, musl) are compiled afresh too. Only downloads are
    shared: the crates (pinned by Cargo.lock checksums), cargo-xwin and the
    pinned MSVC CRT/SDK.
    """
    command_dist(argparse.Namespace(jobs=args.jobs, clean=args.clean,
                                    output=None, url_base=None))
    # The second build reuses this checkout's downloads.
    ensure_cargo_xwin()

    scratch = Path(tempfile.mkdtemp(prefix="idt3-repro-"))
    copy = scratch / "checkout"
    for name in source_files():
        (copy / name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, copy / name)
    for shared in (TOOLS_DIR, XWIN_CACHE):
        shutil.copytree(shared, copy / shared.relative_to(ROOT), symlinks=True)

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
    # build/ is what the former CMake build left behind.
    for directory in (TARGET_DIR, DIST_DIR, ROOT / "build"):
        print(f"removing {directory}", flush=True)
        shutil.rmtree(directory, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subparsers = parser.add_subparsers(dest="command", required=True)

    def add_common(subparser):
        subparser.add_argument("-j", "--jobs", type=int, help="parallel build jobs")
        subparser.add_argument("--clean", action="store_true",
                               help="discard the existing build output first")

    targets = subparsers.add_parser("targets", help="list the supported platforms")
    targets.set_defaults(func=command_targets, toolchain=False)

    version = subparsers.add_parser("version", help="print the project version")
    version.set_defaults(func=command_version, toolchain=False)

    build = subparsers.add_parser("build", help="cross compile for some platforms")
    add_common(build)
    build.add_argument("--debug", action="store_true",
                       help="unoptimized build with debug assertions and overflow checks")
    build.add_argument("-a", "--all", action="store_true", help="build every platform")
    build.add_argument("-t", "--target", action="append", metavar="PLATFORM",
                       help="platform to build (repeatable, default: the host)")
    build.set_defaults(func=command_build, toolchain=True)

    test = subparsers.add_parser("test", help="run the tests on the host")
    add_common(test)
    test.add_argument("--release", action="store_true",
                      help="test the release profile (no overflow checks)")
    test.set_defaults(func=command_test, toolchain=True)

    dist = subparsers.add_parser(
        "dist", help="release build of every platform, collected into ./dist")
    add_common(dist)
    dist.add_argument("-o", "--output", metavar="DIR",
                      help="collect into DIR instead of ./dist")
    dist.add_argument("--url-base", metavar="URL",
                      help=f"also write {MULTITOOL_LOCK} for binaries published "
                           f"under URL (e.g. a GitHub release download URL)")
    dist.set_defaults(func=command_dist, toolchain=True)

    repro = subparsers.add_parser(
        "repro", help="build the dist twice independently and compare the bytes")
    add_common(repro)
    repro.add_argument("--keep", action="store_true",
                       help="keep the second build even when it is identical")
    repro.set_defaults(func=command_repro, toolchain=True)

    clean = subparsers.add_parser("clean", help="remove ./target and ./dist")
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
