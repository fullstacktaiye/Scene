#!/bin/sh
# Build Scene's distro packages in containers.
#
#   scripts/package.sh              every target
#   scripts/package.sh fedora       one of fedora, debian, arch
#
# Containers rather than the host, for two reasons: the packages are built
# against the distribution they target instead of whatever this machine has,
# and each build installs its dependencies from the packaging metadata itself,
# so an incomplete BuildRequires/Build-Depends/makedepends list fails the build
# instead of borrowing something the machine already had.
#
# Everything each build needs is assembled into target/dist/context-TARGET, so
# the container never sees the working tree. Results, and the rpmlint, lintian
# and namcap reports, land in target/packages/TARGET.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

targets=${*:-fedora debian arch}
dist=$root/target/dist
packages=$root/target/packages

for target in $targets; do
    case $target in
        fedora | debian | arch) ;;
        *)
            echo "unknown target: $target (expected fedora, debian or arch)" >&2
            exit 2
            ;;
    esac
done

"$root/scripts/release-tarball.sh"

for target in $targets; do
    context=$dist/context-$target
    output=$packages/$target
    rm -rf "$context" "$output"
    mkdir -p "$context" "$output"

    cp "$dist"/*.tar.gz "$context/"
    cp "$root/packaging/$target/Dockerfile" "$context/Dockerfile"
    case $target in
        fedora) cp "$root/packaging/fedora/scene.spec" "$context/" ;;
        debian)
            cp -r "$root/packaging/debian" "$context/debian"
            rm -f "$context/debian/Dockerfile"
            ;;
        arch) cp "$root/packaging/arch/PKGBUILD" "$context/" ;;
    esac

    echo
    echo "==> $target"
    docker build --progress=plain --target export \
        --output "type=local,dest=$output" "$context"
    rm -rf "$context"

    echo "--- $target results"
    ls -la "$output"
done
