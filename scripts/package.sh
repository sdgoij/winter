#!/bin/sh
# Builds this platform's release and packages it into `dist/`.
#
#   just package
#       dist/winter-<version>-<target>.tar.gz            Linux
#       dist/winter-<version>-<target>/Winter.app         macOS, inside the tar
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
# (ADR 074).
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
# CFBundleVersion is a period-separated list of integers; the display string
# keeps the prerelease suffix that `CFBundleShortVersionString` may hold.
build_version=$(printf '%s' "$version" | sed 's/[^0-9.].*$//; s/\.$//')
[ -n "$build_version" ] || build_version=0
name="winter-$version-$target"
stage="dist/$name"

cargo build --release -p winter

# The bundle Launch Services registers: the executable, the icon the binary
# draws for itself, and the plist that names both. The document type is an
# application, so a double-click starts the shell rather than a terminal.
build_app() {
    app=$1
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

    cp target/release/winter "$app/Contents/MacOS/winter"

    iconset="$app/Contents/Resources/Winter.iconset"
    WINTER_APPICON="$iconset" "$app/Contents/MacOS/winter"
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

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    if [ "${1:-}" = "--app" ]; then
        echo "package: a .app is macOS only" >&2
        exit 1
    fi
    rm -rf "$stage"
    mkdir -p "$stage"
    cp target/release/winter.exe "$stage/winter.exe"
    cp LICENSE.md "$stage/LICENSE.md"
    cp licenses/OpenConsole-MIT.txt "$stage/LICENSE-OpenConsole.txt"
    cp README.md "$stage/README.md"
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/console-host.ps1 -Destination "$stage"
    powershell -NoProfile -ExecutionPolicy Bypass -Command \
        "Compress-Archive -LiteralPath '$(cygpath -m "$stage")' -DestinationPath '$(cygpath -m "$root/dist/$name.zip")' -Force"
    echo "dist/$name.zip"
    ;;
Darwin)
    if [ "${1:-}" = "--app" ]; then
        build_app "dist/Winter.app"
        echo "dist/Winter.app"
        exit 0
    fi
    rm -rf "$stage"
    mkdir -p "$stage"
    build_app "$stage/Winter.app"
    cp LICENSE.md "$stage/LICENSE.md"
    cp README.md "$stage/README.md"
    tar -czf "dist/$name.tar.gz" -C dist "$name"
    echo "dist/$name.tar.gz"
    ;;
*)
    if [ "${1:-}" = "--app" ]; then
        echo "package: a .app is macOS only" >&2
        exit 1
    fi
    rm -rf "$stage"
    mkdir -p "$stage"
    cp target/release/winter "$stage/winter"
    cp LICENSE.md "$stage/LICENSE.md"
    cp README.md "$stage/README.md"
    tar -czf "dist/$name.tar.gz" -C dist "$name"
    echo "dist/$name.tar.gz"
    ;;
esac
