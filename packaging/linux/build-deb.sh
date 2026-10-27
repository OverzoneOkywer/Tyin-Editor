#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

ref_name="${1:?Missing Git ref name}"
ref_type="${2:?Missing Git ref type}"
run_number="${3:?Missing GitHub run number}"

if [[ "$ref_type" == "tag" ]]; then
    version="${ref_name#v}"
else
    version="0.1.0~dev.${run_number}"
fi

if [[ ! "$version" =~ ^[0-9]+(\.[0-9]+){1,2}([.+~:-][A-Za-z0-9.+~:-]+)*$ ]]; then
    printf 'Invalid Debian package version: %s\n' "$version" >&2
    exit 1
fi

mkdir -p dist
package_root="$(mktemp -d dist/tyin-editor-package.XXXXXX)"
trap 'rm -rf -- "$package_root"' EXIT

install -D -m 0755 target/release/tyin-app \
    "$package_root/usr/bin/tyin-editor"
install -D -m 0644 packaging/linux/tyin-editor.desktop \
    "$package_root/usr/share/applications/tyin-editor.desktop"
install -D -m 0644 assets/tyin-mark.svg \
    "$package_root/usr/share/icons/hicolor/scalable/apps/tyin-editor.svg"
mkdir -p "$package_root/DEBIAN"
sed "s/@VERSION@/$version/" packaging/linux/control \
    > "$package_root/DEBIAN/control"
find "$package_root" -type d -exec chmod 0755 {} +

dpkg-deb --root-owner-group --build \
    "$package_root" "dist/tyin-editor_${version}_amd64.deb"
