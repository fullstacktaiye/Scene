#!/bin/sh
# List every crate Scene links statically, with its licence, to stdout.
#
# Scene ships one binary with its dependencies compiled in, so every package
# has to say what is inside it. scripts/release-tarball.sh runs this once and
# puts the result in the release tarball as LICENSE.dependencies, which each
# distro package then installs beside Scene's own LICENSE. Fedora regenerates
# the same file with %{cargo_license}; the others install what was shipped.
#
# It reads Cargo.lock through cargo metadata, so it describes the resolved
# build rather than the version ranges in Cargo.toml.

set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

host=$(rustc --version --verbose | sed -n 's/^host: //p')
cargo metadata --locked --format-version 1 --filter-platform "$host" | python3 -c '
import json, sys

meta = json.load(sys.stdin)
root = {p["id"] for p in meta["packages"] if p["name"] == "scene"}
crates = sorted(
    ((p["name"], p["version"], p.get("license") or "see " + (p.get("license_file") or "the crate"))
     for p in meta["packages"] if p["id"] not in root),
    key=lambda c: (c[0].lower(), c[1]),
)

print("Scene is MIT; see the LICENSE file beside this one.")
print()
print("It is built as a single binary, so the %d crates below are compiled" % len(crates))
print("into it. This list is generated from Cargo.lock, not from Cargo.toml,")
print("so it names the versions that were actually built.")
print()
width = max(len(name) for name, _, _ in crates)
for name, version, license in crates:
    print("%-*s  %-10s  %s" % (width, name, version, license))
'
