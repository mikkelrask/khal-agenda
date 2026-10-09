#!/usr/bin/env bash
set -euo pipefail
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libgtk-4-dev libwayland-dev wayland-protocols meson ninja-build patchelf rpm
curl -fL --retry 3 https://github.com/wmww/gtk4-layer-shell/archive/refs/tags/v1.3.0.tar.gz -o /tmp/gtk4-layer-shell.tar.gz
printf '%s  %s\n' '1ebb01ab14e98afd1727f68f64981c37bd23305b1f131f5667c02b94cf593192' /tmp/gtk4-layer-shell.tar.gz | sha256sum -c -
tar -xzf /tmp/gtk4-layer-shell.tar.gz -C /tmp
meson setup /tmp/gtk4-layer-shell-1.3.0/build /tmp/gtk4-layer-shell-1.3.0 --prefix=/usr -Dexamples=false -Dtests=false -Ddocs=false -Dintrospection=false -Dvapi=false
ninja -C /tmp/gtk4-layer-shell-1.3.0/build
sudo ninja -C /tmp/gtk4-layer-shell-1.3.0/build install
sudo ldconfig
