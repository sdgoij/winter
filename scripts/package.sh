#!/bin/sh
# Builds this platform's release and packages it into `dist/`.
#
#   just package
#       dist/winter-<version>-<target>.tar.gz    Linux, macOS
#       dist/winter-<version>-<target>.zip       Windows
#
# `<version>` is `WINTER_VERSION` with a leading `v` stripped when the
# environment supplies one -- a tag, in CI, so a preview and the final release
# cannot ship assets with the same name -- and the version in Cargo.toml
# otherwise. The target comes from rustc, so an archive's name always matches
# the binary inside it and two platforms can never write the same file.
# Windows also gets the console host and the licences it travels with
# (ADR 055); elsewhere the host is the system's own.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

version=${WINTER_VERSION:-}
version=${version#v}
if [ -z "$version" ]; then
    version=$(awk '/^\[workspace.package\]/ { found = 1 } found && /^version *=/ { gsub(/[^0-9.]/, "", $3); print $3; exit }' Cargo.toml)
    if [ -z "${version:-}" ]; then
        echo "package: no version under [workspace.package] in Cargo.toml" >&2
        exit 1
    fi
fi

target=$(rustc -vV | awk '/^host:/ { print $2 }')
name="winter-$version-$target"
stage="dist/$name"

cargo build --release -p winter

rm -rf "$stage"
mkdir -p "$stage"

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    cp target/release/winter.exe "$stage/winter.exe"
    cp LICENSE.md "$stage/LICENSE.md"
    cp licenses/OpenConsole-MIT.txt "$stage/LICENSE-OpenConsole.txt"
    cp README.md "$stage/README.md"
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/console-host.ps1 -Destination "$stage"
    powershell -NoProfile -ExecutionPolicy Bypass -Command \
        "Compress-Archive -LiteralPath '$(cygpath -m "$stage")' -DestinationPath '$(cygpath -m "$root/dist/$name.zip")' -Force"
    echo "dist/$name.zip"
    ;;
*)
    cp target/release/winter "$stage/winter"
    cp LICENSE.md "$stage/LICENSE.md"
    cp README.md "$stage/README.md"
    tar -czf "dist/$name.tar.gz" -C dist "$name"
    echo "dist/$name.tar.gz"
    ;;
esac
