#!/usr/bin/env python3
"""Re-creates third_party/compressonator from the pinned upstream release.

Only the files that the CMP_Core BC1-BC7 encoders need are vendored, byte for
byte as published. The local fixes are applied by build.rs at build time, so
the vendored tree can always be compared against upstream with this script:

    python3 -I tools/vendor_compressonator.py [--tarball V4.6.0.tar.gz]

The script fails if the tarball does not match the pinned checksum or if any
expected file is missing from it.
"""

import argparse
import hashlib
import io
import shutil
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

URL = "https://github.com/GPUOpen-Tools/compressonator/archive/refs/tags/V4.6.0.tar.gz"
SHA256 = "4e64d84336689763a107feff76275d59080a329d14242c55e2273f7319530743"

# Include closure of the sources compiled by build.rs, plus the license that
# covers them. Keep this list explicit so upstream changes cannot slip in.
FILES = [
    "license/corelicense.txt",
    "applications/_libs/cmp_math/cmp_math_common.cpp",
    "applications/_libs/cmp_math/cmp_math_common.h",
    "applications/_libs/cmp_math/cpu_extensions.h",
    "cmp_core/shaders/bc1_cmp.h",
    "cmp_core/shaders/bc1_common_kernel.h",
    "cmp_core/shaders/bc1_encode_kernel.cpp",
    "cmp_core/shaders/bc1_encode_kernel.h",
    "cmp_core/shaders/bc2_encode_kernel.cpp",
    "cmp_core/shaders/bc2_encode_kernel.h",
    "cmp_core/shaders/bc3_encode_kernel.cpp",
    "cmp_core/shaders/bc3_encode_kernel.h",
    "cmp_core/shaders/bc4_encode_kernel.cpp",
    "cmp_core/shaders/bc4_encode_kernel.h",
    "cmp_core/shaders/bc5_encode_kernel.cpp",
    "cmp_core/shaders/bc5_encode_kernel.h",
    "cmp_core/shaders/bc6_common_encoder.h",
    "cmp_core/shaders/bc6_encode_kernel.cpp",
    "cmp_core/shaders/bc6_encode_kernel.h",
    "cmp_core/shaders/bc7_cmpmsc.h",
    "cmp_core/shaders/bc7_common_encoder.h",
    "cmp_core/shaders/bc7_encode_kernel.cpp",
    "cmp_core/shaders/bc7_encode_kernel.h",
    "cmp_core/shaders/bcn_common_api.h",
    "cmp_core/shaders/bcn_common_kernel.h",
    "cmp_core/shaders/common_def.h",
    "cmp_core/source/cmp_core.cpp",
    "cmp_core/source/cmp_core.h",
    "cmp_core/source/cmp_math_func.h",
    "cmp_core/source/cmp_math_vec4.h",
    "cmp_core/source/core_simd.h",
]


def fetch(tarball):
    """Returns the release tarball bytes after verifying the checksum."""
    if tarball:
        data = Path(tarball).read_bytes()
    else:
        with urllib.request.urlopen(URL) as resp:
            data = resp.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != SHA256:
        sys.exit(f"checksum mismatch: got {digest}, expected {SHA256}")
    return data


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--tarball", help="use this local copy instead of downloading")
    args = ap.parse_args()

    dest = Path(__file__).resolve().parent.parent / "third_party" / "compressonator"
    data = fetch(args.tarball)

    with tempfile.TemporaryDirectory() as tmp:
        staging = Path(tmp) / "compressonator"
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            members = {}
            for m in tar.getmembers():
                # Strip the "compressonator-4.6.0/" top-level directory.
                parts = m.name.split("/", 1)
                if len(parts) == 2 and m.isfile():
                    members[parts[1]] = m
            for name in FILES:
                m = members.get(name)
                if m is None:
                    sys.exit(f"missing from tarball: {name}")
                out = staging / name
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(tar.extractfile(m).read())
        shutil.copy(Path(__file__).resolve().parent.parent / "third_party" / "compressonator.README.md",
                    staging / "README.md")
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(staging, dest)
    print(f"vendored {len(FILES)} files into {dest}")


if __name__ == "__main__":
    main()
