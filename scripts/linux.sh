#!/bin/sh
# Runs the Linux shell (ADRs 061, 075): the GTK 4 shell by default, or the egui
# fallback with `--no-default-features`.
#
#   just linux                          # on Linux: this machine's build
#   just linux FedoraLinux-44           # on Windows: in that WSL distro
#   just linux FedoraLinux-44 --release # that distro, optimised
#   just linux --release                # on Linux: anything else cargo takes
#
# The GTK shell needs GTK 4.10 development headers. On Windows those live in
# WSL, so the first argument names the distro to run in; `WSL_DISTRO` does the
# same from the environment. Neither is defaulted: a distro's name is a fact
# about a machine, and a guess that is wrong teaches nothing.
#
# A build under `/mnt` means the repository is on a Windows drive, so the guest
# writes its artefacts to its own home instead of on top of the Windows ones.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

run() {
    case "$PWD" in
    /mnt/*) CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$HOME/winterm-target"} ;;
    esac
    export CARGO_TARGET_DIR
    cargo run -p winter "$@"
}

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    distro=${WSL_DISTRO:-}
    # A leading `-` is a cargo flag, not a distro name.
    case "${1:-}" in
    "" | -*) ;;
    *)
        distro=$1
        shift
        ;;
    esac

    if [ -z "$distro" ]; then
        echo "linux: name the WSL distro that has the GTK 4 headers:" >&2
        echo "       just linux <distro> [cargo args...]" >&2
        exit 1
    fi

    # The guest converts the Windows path the shell reports and runs this same
    # script, which then takes the Linux branch. A login shell is what puts the
    # guest's `~/.cargo/bin` on the PATH.
    wsl.exe -d "$distro" -e sh -lc \
        'cd "$(wslpath -u "$1")" && shift && exec sh scripts/linux.sh "$@"' \
        sh "$(cygpath -w "$PWD")" "$@"
    ;;
*)
    run "$@"
    ;;
esac
