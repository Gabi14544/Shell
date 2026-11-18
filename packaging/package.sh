#!/bin/sh
set -eu
destination=$(realpath -m "${1:-.}")
cd "$(dirname "$0")/.."
if [ "$(dpkg --print-architecture)" != amd64 ]; then
    printf 'Este paquete está preparado para Linux x86_64.\n' >&2
    exit 1
fi
if [ "${BRASA_USE_BUILT_BINARY:-0}" != 1 ]; then
    cargo build --release --locked --jobs "${BRASA_BUILD_JOBS:-2}"
fi
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
stage=$(mktemp -d)
chmod 0755 "$stage"
trap 'rm -rf "$stage"' EXIT HUP INT TERM

install -d "$stage/DEBIAN"
cat > "$stage/DEBIAN/control" <<CONTROL
Package: brasa-terminal
Version: $version
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Brasa Terminal <brasa@localhost>
Depends: libc6 (>= 2.39), libgtk-3-0t64, libvte-2.91-0 (>= 0.70), libgtksourceview-4-0, fonts-hack, fonts-dejavu-core
Recommends: python3-venv
Description: Terminal de escritorio compacta con editor y extensiones de lenguaje
 Brasa utiliza GTK, VTE y GtkSourceView. Inicia servidores de lenguaje
 al abrir un archivo compatible en el editor.
CONTROL

install -Dm755 "${CARGO_TARGET_DIR:-target}/release/brasa" "$stage/usr/bin/brasa"
install -Dm644 packaging/ar.brasa.Terminal.desktop "$stage/usr/share/applications/ar.brasa.Terminal.desktop"
install -Dm644 packaging/ar.brasa.Terminal.png "$stage/usr/share/icons/hicolor/128x128/apps/ar.brasa.Terminal.png"
install -Dm644 assets/styles/brasa.xml "$stage/usr/share/brasa/styles/brasa.xml"
install -Dm644 README.md "$stage/usr/share/doc/brasa-terminal/README.md"
mkdir -p "$destination"
dpkg-deb --root-owner-group --build "$stage" "$destination/brasa-terminal_${version}_amd64.deb"
