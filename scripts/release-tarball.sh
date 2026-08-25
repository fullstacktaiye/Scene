#!/bin/sh
# Build the two tarballs every Scene package is made from:
#
#   target/dist/scene-VERSION.tar.gz         the sources
#   target/dist/scene-VERSION-vendor.tar.gz  every crate they depend on
#
# The vendor tarball exists because all three package builds run offline. A
# packaging build that downloads from crates.io is not reproducible and is not
# allowed by Fedora's guidelines, so the dependencies are resolved once, here,
# from Cargo.lock.
#
# Both tarballs are byte-for-byte reproducible from the same tree: the file
# list is sorted, ownership is numeric root, every mtime is the last commit's
# timestamp, and gzip is told not to record its own.
#
# The source list is what git tracks plus what it would track — everything not
# ignored — so packaging a working tree does not silently omit a file that has
# not been committed yet.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
name=$(sed -n 's/^name = "\(.*\)"$/\1/p' Cargo.toml | head -1)
dist=$root/target/dist
stamp=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}

rm -rf "$dist"
mkdir -p "$dist"

echo "Scene $version, source date @$stamp"

stage=$dist/$name-$version
mkdir -p "$stage"
git ls-files --cached --others --exclude-standard | LC_ALL=C sort > "$dist/files"
while IFS= read -r file; do
    cp --parents --preserve=mode "$file" "$stage/"
done < "$dist/files"

# Generated rather than tracked: every package installs it, and it has to
# describe the crates Cargo.lock resolved at the moment the release was cut.
"$root/scripts/dependency-licenses.sh" > "$stage/LICENSE.dependencies"

tar --create --file - --directory "$dist" "$name-$version" \
    --owner=0 --group=0 --numeric-owner --mode=go-w \
    --mtime="@$stamp" --sort=name --format=gnu \
    | gzip -n -9 > "$dist/$name-$version.tar.gz"
rm -rf "$stage"
echo "  sources  $dist/$name-$version.tar.gz ($(wc -l < "$dist/files") files, plus LICENSE.dependencies)"

# --locked, so the vendored set is exactly what Cargo.lock resolved and not
# whatever crates.io happens to offer today.
cargo vendor --locked --quiet "$dist/vendor" > "$dist/vendor-config.toml"
tar --create --file - --directory "$dist" vendor \
    --owner=0 --group=0 --numeric-owner --mode=go-w \
    --mtime="@$stamp" --sort=name --format=gnu \
    | gzip -n -9 > "$dist/$name-$version-vendor.tar.gz"
rm -rf "$dist/vendor" "$dist/files"
echo "  vendor   $dist/$name-$version-vendor.tar.gz"

echo
echo "sha256, for a release's checksums:"
(cd "$dist" && sha256sum "$name-$version.tar.gz" "$name-$version-vendor.tar.gz" | sed 's/^/  /')
