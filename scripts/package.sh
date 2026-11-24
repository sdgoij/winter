#!/bin/sh
# Builds this platform's release and packages it into `dist/`.
#
#   just package
#       dist/winter-<version>-<target>.tar.gz            Linux
#       dist/winter-<version>-<target>.tar.gz            macOS, one per arch
#       dist/winter-<version>-<target>.zip                Windows
#
#   just app
#       dist/Winter.app                                   macOS only
#
# `<version>` is `WINTER_VERSION` with a leading `v` stripped when the
# environment supplies one -- a tag, in CI, so a preview and the final release
# cannot ship assets with the same name -- and the version in Cargo.toml
# otherwise. The target comes from rustc, so an archive's name always matches
# the binary inside it and two platforms can never write the same file.
# Windows also gets the console host and the licences it travels with
# (ADR 055); elsewhere the host is the system's own. macOS gets an app bundle,
# because a bare executable opened from Finder starts under a Terminal window
# (ADR 074), and both architectures, because the Intel runner cross-builds the
# Apple silicon archive instead of waiting for an arm64 macOS runner.
#
# `WINTER_TARGET=<target>` packages one target instead of the default set.
#
# Linux ships an app rather than a bare binary -- a desktop entry, the icon the
# binary draws and an installer that needs no root (ADR 076).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

platform=$(uname -s)

version=${WINTER_VERSION:-}
version=${version#v}
if [ -z "$version" ]; then
    version=$(awk '/^\[workspace.package\]/ { found = 1 } found && /^version *=/ { gsub(/[^0-9.]/, "", $3); print $3; exit }' Cargo.toml)
    if [ -z "${version:-}" ]; then
        echo "package: no version under [workspace.package] in Cargo.toml" >&2
        exit 1
    fi
fi

# `CARGO_TARGET_DIR` moves the build directory; an agent or a WSL checkout
# usually sets it, so every binary path below follows it.
target_dir=${CARGO_TARGET_DIR:-$root/target}
host=$(rustc -vV | awk '/^host:/ { print $2 }')
# CFBundleVersion is a period-separated list of integers; the display string
# keeps the prerelease suffix that `CFBundleShortVersionString` may hold.
build_version=$(printf '%s' "$version" | sed 's/[^0-9.].*$//; s/\.$//')
[ -n "$build_version" ] || build_version=0

# The bundle Launch Services registers: the executable, the icon the binary
# draws for itself, and the plist that names both. The document type is an
# application, so a double-click starts the shell rather than a terminal.
# `icon_binary` is a host build, because a cross-compiled binary cannot run.
build_app() {
    app=$1
    binary=$2
    icon_binary=$3
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

    cp "$binary" "$app/Contents/MacOS/winter"

    iconset="$app/Contents/Resources/Winter.iconset"
    WINTER_APPICON="$iconset" "$icon_binary"
    iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
    rm -rf "$iconset"

    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Winter</string>
    <key>CFBundleExecutable</key>
    <string>winter</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.sdgoij.winter</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Winter</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$build_version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
PLIST

    # Ad-hoc signing gives the bundle one identity, which is all a locally built
    # app needs. A Developer ID and notarization are the release task (ADR 074).
    if command -v codesign >/dev/null 2>&1; then
        codesign --force --sign - "$app" >/dev/null 2>&1 || true
    fi
}

if [ "${1:-}" = "--app" ]; then
    if [ "$platform" != Darwin ]; then
        echo "package: a .app is macOS only" >&2
        exit 1
    fi
    cargo build --release -p winter
    build_app "dist/Winter.app" "$target_dir/release/winter" "$target_dir/release/winter"
    echo "dist/Winter.app"
    exit 0
fi

# The targets this run packages.
if [ -n "${WINTER_TARGET:-}" ]; then
    targets=$WINTER_TARGET
elif [ "$platform" = Darwin ]; then
    # Both slices are in the SDK, so clang links either architecture from this
    # host. Building the other one here is what keeps a release off the arm64
    # macOS runner pool.
    case "$host" in
    aarch64-*) targets="$host x86_64-apple-darwin" ;;
    *) targets="$host aarch64-apple-darwin" ;;
    esac
else
    targets=$host
fi

check_target() {
    if command -v rustup >/dev/null 2>&1 && ! rustup target list --installed | grep -qx "$1"; then
        echo "package: $1 is not installed; run 'rustup target add $1'" >&2
        exit 1
    fi
}

cargo build --release -p winter
icon_binary="$target_dir/release/winter"

for target in $targets; do
    check_target "$target"

    if [ "$target" = "$host" ]; then
        binary="$target_dir/release/winter"
    else
        cargo build --release -p winter --target "$target"
        binary="$target_dir/$target/release/winter"
    fi
    case "$target" in
    *windows*) binary="$binary.exe" ;;
    esac

    name="winter-$version-$target"
    stage="$root/dist/$name"
    rm -rf "$stage"
    mkdir -p "$stage"

    case "$platform" in
    MINGW* | MSYS* | CYGWIN*)
        cp "$binary" "$stage/winter.exe"
        cp LICENSE.md "$stage/LICENSE.md"
        cp licenses/OpenConsole-MIT.txt "$stage/LICENSE-OpenConsole.txt"
        cp README.md "$stage/README.md"
        powershell -NoProfile -ExecutionPolicy Bypass -File scripts/console-host.ps1 -Destination "$stage"
        powershell -NoProfile -ExecutionPolicy Bypass -Command \
            "Compress-Archive -LiteralPath '$(cygpath -m "$stage")' -DestinationPath '$(cygpath -m "$root/dist/$name.zip")' -Force"
        echo "dist/$name.zip"
        ;;
    Darwin)
        build_app "$stage/Winter.app" "$binary" "$icon_binary"
        cp LICENSE.md "$stage/LICENSE.md"
        cp README.md "$stage/README.md"
        tar -czf "$root/dist/$name.tar.gz" -C "$root/dist" "$name"
        echo "dist/$name.tar.gz"
        ;;
    Linux)
        # The archive is an app, not a bare binary: launching it from the menu
        # is what the other two platforms' shells get from their packaging
        # (ADR 076).
        cp "$binary" "$stage/winter"
        cp LICENSE.md "$stage/LICENSE.md"
        cp README.md "$stage/README.md"
        cp packaging/linux/install.sh "$stage/install.sh"
        chmod +x "$stage/install.sh"
        cp packaging/linux/org.winter.Winter.desktop "$stage/org.winter.Winter.desktop"
        # The icon is drawn rather than committed, so the menu's icon and the
        # window's cannot drift (ADRs 058, 076).
        cargo run --quiet --release -p winter --example write-icon -- "$stage/winter.png"
        # The entry is what a user clicks; a typo in it is a broken install.
        if command -v desktop-file-validate >/dev/null 2>&1; then
            desktop-file-validate "$stage/org.winter.Winter.desktop"
        fi
        tar -czf "$root/dist/$name.tar.gz" -C "$root/dist" "$name"
        echo "dist/$name.tar.gz"
        ;;
    *)
        cp "$binary" "$stage/winter"
        cp LICENSE.md "$stage/LICENSE.md"
        cp README.md "$stage/README.md"
        tar -czf "$root/dist/$name.tar.gz" -C "$root/dist" "$name"
        echo "dist/$name.tar.gz"
        ;;
    esac
done
