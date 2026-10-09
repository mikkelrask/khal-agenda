#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
arch=$(uname -m)
output="$PWD/dist"
stage="$output/stage"
mkdir -p "$output"
rm -rf "$stage"
install -Dm755 target/release/khal-agenda "$stage/usr/bin/khal-agenda"
install -Dm644 data/khal-agenda.desktop "$stage/usr/share/applications/khal-agenda.desktop"
install -Dm644 data/khal-agenda.svg "$stage/usr/share/icons/hicolor/scalable/apps/khal-agenda.svg"
install -Dm644 LICENSE "$stage/usr/share/licenses/khal-agenda/LICENSE"
install -Dm644 README.md "$stage/usr/share/doc/khal-agenda/README.md"
for image in docs/screenshots/*.png; do
    install -Dm644 "$image" "$stage/usr/share/doc/khal-agenda/$image"
done
layer_lib=$(pkg-config --variable=libdir gtk4-layer-shell-0)/libgtk4-layer-shell.so.0
install -Dm755 "$(readlink -f "$layer_lib")" "$stage/usr/lib/khal-agenda/libgtk4-layer-shell.so.0"
install -Dm644 packaging/gtk4-layer-shell.LICENSE "$stage/usr/share/licenses/khal-agenda/gtk4-layer-shell.LICENSE"
patchelf --set-rpath "\$ORIGIN/../lib/khal-agenda" "$stage/usr/bin/khal-agenda"
tar -C "$stage/usr" -czf "$output/khal-agenda-$version-linux-$arch.tar.gz" .

(cd "$output" && sha256sum ./*.tar.gz > SHA256SUMS)
