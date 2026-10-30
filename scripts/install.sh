#!/bin/sh
# Installs the Riflesso X command line (`riflesso`) for the current user on Linux:
#
#   curl -fsSL https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.sh | sh
#
# Downloads the command line from the latest release, checks it against the
# published SHA-256 and puts it in ~/.local/bin (its data goes to
# ~/.local/share/riflesso-x-cli). Nothing needs root. To remove it:
#
#   rm ~/.local/bin/riflesso; rm -r ~/.local/share/riflesso-x-cli
#
# RIFLESSO_INSTALL_DIR picks another folder; RIFLESSO_CLI_TAR installs a local
# .tar.gz instead of downloading (to test a build that is not published yet).

set -eu

repo=kannarozzi/Riflesso-X
dir=${RIFLESSO_INSTALL_DIR:-"$HOME/.local/bin"}

fail() {
    printf 'Error: %s\n' "$1" >&2
    exit 1
}

case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) ;;
    *) fail "this script installs the Linux x86_64 build; this is $(uname -s) $(uname -m)." ;;
esac

# curl or wget, whichever the system has.
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL -A riflesso-install "$1"; }
    save() { curl -fsSL -A riflesso-install -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- -U riflesso-install "$1"; }
    save() { wget -q -U riflesso-install -O "$2" "$1"; }
else
    fail "neither curl nor wget is installed."
fi

temp=$(mktemp -d)
trap 'rm -rf "$temp"' EXIT
archive="$temp/riflesso.tar.gz"

if [ -n "${RIFLESSO_CLI_TAR:-}" ]; then
    cp "$RIFLESSO_CLI_TAR" "$archive"
    version="local build"
else
    echo "Looking for the latest Riflesso X release..."
    release=$(fetch "https://api.github.com/repos/$repo/releases/latest") || fail "cannot reach GitHub."
    # The download links of the release's files, one per line (no JSON tool needed).
    links=$(printf '%s' "$release" | grep -o '"browser_download_url": *"[^"]*"' | sed 's/.*"\(https[^"]*\)"$/\1/')
    url=$(printf '%s\n' "$links" | grep '_linux-x64-cli\.tar\.gz$' | head -n 1)
    sums_url=$(printf '%s\n' "$links" | grep '/SHA256SUMS\.txt$' | head -n 1)
    version=$(printf '%s' "$release" | grep -o '"tag_name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
    [ -n "$url" ] || fail "the latest release ($version) has no Linux command-line build."
    name=${url##*/}

    echo "Downloading $name..."
    save "$url" "$archive" || fail "the download failed."
    if [ -n "$sums_url" ]; then
        save "$sums_url" "$temp/SHA256SUMS.txt" || fail "cannot download the checksums."
        expected=$(grep " $name\$" "$temp/SHA256SUMS.txt" | cut -d ' ' -f 1)
        actual=$(sha256sum "$archive" | cut -d ' ' -f 1)
        if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
            fail "the download does not match its published SHA-256 checksum. Nothing was installed."
        fi
    fi
fi

tar -xzf "$archive" -C "$temp"
program=$(find "$temp" -type f -name riflesso | head -n 1)
[ -n "$program" ] || fail "riflesso is missing from the download."

mkdir -p "$dir"
# Copied next to the old one, then moved over it: a running copy keeps working.
cp "$program" "$dir/.riflesso.new"
chmod 755 "$dir/.riflesso.new"
mv -f "$dir/.riflesso.new" "$dir/riflesso"

echo
echo "Riflesso X command line installed ($version) in $dir"

# The program needs GStreamer; tell which packages are missing, per distribution.
missing=$(ldd "$dir/riflesso" 2>/dev/null | grep 'not found' || true)
parser_missing=false
if command -v gst-inspect-1.0 >/dev/null 2>&1 && ! gst-inspect-1.0 h264parse >/dev/null 2>&1; then
    parser_missing=true
fi
if [ -n "$missing" ] || [ "$parser_missing" = true ]; then
    # shellcheck disable=SC1091  # the system's file, read at run time
    family=$(. /etc/os-release 2>/dev/null && echo "${ID:-} ${ID_LIKE:-}" || true)
    case "$family" in
        *fedora*|*rhel*)
            command="sudo dnf install gstreamer1-plugins-base gstreamer1-plugins-bad-free gstreamer1-plugin-openh264" ;;
        *debian*|*ubuntu*)
            command="sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-bad gstreamer1.0-libav" ;;
        *arch*)
            command="sudo pacman -S gst-plugins-base gst-plugins-bad gst-libav" ;;
        *suse*)
            command="sudo zypper install gstreamer-plugins-base gstreamer-plugins-bad gstreamer-plugins-libav" ;;
        *)
            command="install GStreamer with its base and bad plugins, and gst-libav" ;;
    esac
    echo
    echo "Riflesso X needs GStreamer to show the video. Install it with:"
    echo "  $command"
fi

case ":$PATH:" in
    *":$dir:"*) echo 'Run "riflesso" to start receiving, or "riflesso --help" for the options.' ;;
    *)
        echo
        echo "$dir is not in your PATH yet: log out and back in (most distributions"
        echo "add it once it exists), or run it as $dir/riflesso."
        ;;
esac
