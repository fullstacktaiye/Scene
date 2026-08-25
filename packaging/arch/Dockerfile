# Build Scene's Arch package in a clean container. makepkg installs the
# PKGBUILD's own depends and makedepends, so a missing one fails the build
# rather than being covered by whatever the image carried.
#
#   docker build -f Dockerfile --target export --output type=local,dest=OUT .
#
# The build context is target/dist, which scripts/release-tarball.sh fills,
# plus this packaging directory's PKGBUILD.

FROM archlinux:latest AS build
RUN pacman -Syu --noconfirm --needed base-devel namcap && rm -rf /var/cache/pacman/pkg/*
RUN useradd --create-home build && printf 'build ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/build

USER build
WORKDIR /home/build
COPY --chown=build:build *.tar.gz PKGBUILD ./
RUN makepkg --syncdeps --noconfirm
RUN namcap PKGBUILD scene-*.pkg.tar.zst > /home/build/namcap.txt 2>&1 || true
RUN cat /home/build/namcap.txt

FROM scratch AS export
COPY --from=build /home/build/*.pkg.tar.zst /
COPY --from=build /home/build/namcap.txt /
