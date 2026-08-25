# Build Scene's .deb in a clean Debian container. The build dependencies are
# installed from debian/control through mk-build-deps rather than listed here,
# so an incomplete Build-Depends fails the build instead of quietly using
# something the image already had.
#
# Unstable rather than stable, and not by preference: the gtk4-rs 0.11 crate
# tree requires rustc 1.92, Debian 13 (trixie) ships 1.85, and trixie-backports
# has nothing newer — verified 2026-08-24. Debian unstable ships 1.95. On
# trixie the package can still be built with a rustup toolchain; it cannot be
# built with the distribution's own.
#
#   docker build -f Dockerfile --target export --output type=local,dest=OUT .
#
# The build context is target/dist, which scripts/release-tarball.sh fills,
# plus this packaging directory as debian/.

FROM debian:unstable AS build
ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential debhelper devscripts equivs lintian \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY *.tar.gz ./
COPY debian/ /build/debian/
# The version is read from the tarball rather than written here, and /build/source
# is a stable name for it so the rest of this file does not have to know it.
RUN set -eu; \
    tarball=$(ls scene-*.tar.gz | grep -v -- -vendor); \
    tar xf "$tarball"; \
    unpacked=$(ls -d scene-*/); \
    tar xf scene-*-vendor.tar.gz -C "$unpacked"; \
    mv debian "$unpacked/debian"; \
    ln -s "/build/$unpacked" /build/source

WORKDIR /build/source
RUN apt-get update \
    && mk-build-deps --install --remove --tool 'apt-get -y --no-install-recommends' debian/control \
    && rm -rf /var/lib/apt/lists/*
RUN dpkg-buildpackage --unsigned-source --unsigned-changes --build=binary
RUN lintian --no-tag-display-limit /build/*.changes > /build/lintian.txt 2>&1 || true
RUN cat /build/lintian.txt

FROM scratch AS export
COPY --from=build /build/*.deb /
COPY --from=build /build/lintian.txt /
