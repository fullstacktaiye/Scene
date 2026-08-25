# Build Scene's RPM the way a review environment would: the spec builds a
# source package first, and the binary build then installs *only* what that
# SRPM declares as BuildRequires. An incomplete BuildRequires list fails here
# rather than silently borrowing a package the image happened to carry.
#
#   docker build -f Dockerfile --target export --output type=local,dest=OUT .
#
# The build context is target/dist, which scripts/release-tarball.sh fills.

FROM fedora:44 AS srpm
RUN dnf -y install rpm-build rpmdevtools && dnf clean all
RUN useradd --create-home build
USER build
WORKDIR /home/build
RUN rpmdev-setuptree
COPY --chown=build:build *.tar.gz rpmbuild/SOURCES/
COPY --chown=build:build scene.spec rpmbuild/SPECS/
RUN rpmbuild -bs rpmbuild/SPECS/scene.spec

FROM fedora:44 AS rpm
RUN dnf -y install 'dnf-command(builddep)' rpm-build rpmlint && dnf clean all
COPY --from=srpm /home/build/rpmbuild/SRPMS/*.src.rpm /srpm/
RUN dnf -y builddep /srpm/*.src.rpm && dnf clean all
RUN useradd --create-home build
USER build
WORKDIR /home/build
RUN rpmbuild --rebuild /srpm/*.src.rpm
# rpmlint reports on the spec, the source package and the binary package. Its
# findings are exported rather than swallowed, so a build that produced an RPM
# with problems cannot be mistaken for a clean one.
RUN rpmlint /srpm/*.src.rpm rpmbuild/RPMS/*/*.rpm > /home/build/rpmlint.txt 2>&1 || true
RUN cat /home/build/rpmlint.txt

FROM scratch AS export
COPY --from=rpm /home/build/rpmbuild/RPMS/*/*.rpm /
COPY --from=srpm /home/build/rpmbuild/SRPMS/*.src.rpm /
COPY --from=rpm /home/build/rpmlint.txt /
