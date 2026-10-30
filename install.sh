#!/bin/sh
# Installs the ready-made kova of the latest release, for this user only (no administrator rights):
#
#   curl -fsSL https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.sh | sh
#
# Into ~/.local/bin, or the folder `KOVA_INSTALL` names. Nothing else is needed to use kova.
set -e
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) name=linux-x64 ;;
    Linux-aarch64 | Linux-arm64) name=linux-arm64 ;;
    Darwin-*) name=mac ;;
    *) echo "kova: no ready-made kova for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac
dir="${KOVA_INSTALL:-$HOME/.local/bin}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "https://github.com/Thibault-Savenkoff/kova/releases/latest/download/kova-$name.tar.gz" -o "$tmp/kova.tar.gz"
tar -xzf "$tmp/kova.tar.gz" -C "$tmp"
mkdir -p "$dir"
mv -f "$tmp/kova" "$dir/kova"
echo "✓ $("$dir/kova" --version) installed: $dir/kova"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "note: $dir is not in your PATH: add it (for example in ~/.profile) to type \`kova\`" ;;
esac
