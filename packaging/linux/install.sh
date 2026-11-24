#!/bin/sh
# Installs Winter for the current user, or removes it again.
#
#   ./install.sh                install into ~/.local (no root needed)
#   ./install.sh --uninstall    remove what it installed
#
# `BINDIR` and `DATADIR` choose the destinations; the defaults are the ones the
# XDG base-directory spec and most distributions' user installs already use.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
bindir=${BINDIR:-$HOME/.local/bin}
datadir=${DATADIR:-${XDG_DATA_HOME:-$HOME/.local/share}}
applications=$datadir/applications
icons=$datadir/icons/hicolor/256x256/apps

# The desktop database and the icon cache are caches: if their tools are not
# installed, the entry still appears on the next login.
refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$datadir/icons/hicolor" || true
    fi
}

case ${1:-} in
--uninstall)
    rm -f "$bindir/winter" "$applications/org.winter.Winter.desktop" "$icons/winter.png"
    refresh
    echo "removed winter from $bindir and $datadir"
    exit 0
    ;;
"") ;;
*)
    echo "install: unknown argument $1" >&2
    echo "usage: install.sh [--uninstall]" >&2
    exit 2
    ;;
esac

install -D -m 755 "$here/winter" "$bindir/winter"
install -D -m 644 "$here/org.winter.Winter.desktop" "$applications/org.winter.Winter.desktop"
install -D -m 644 "$here/winter.png" "$icons/winter.png"
refresh

echo "installed $bindir/winter"
echo "Winter is in the application menu"
case ":$PATH:" in
*":$bindir:"*) ;;
*) echo "note: $bindir is not on PATH, so the \`winter\` command needs the full path" ;;
esac
