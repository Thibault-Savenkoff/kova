#!/bin/sh
# Installs the `.kova` file type for the current user (no root needed).
set -e
here=$(dirname "$0")
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/mime/packages" "$data/icons/hicolor/scalable/mimetypes"
cp "$here/kova.xml" "$data/mime/packages/kova.xml"
cp "$here/../vscode/icons/kova.svg" "$data/icons/hicolor/scalable/mimetypes/text-x-kova.svg"
update-mime-database "$data/mime"
gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true
