#!/bin/bash
# Linux packages (run in the Ubuntu-22.04 based container):
#   cd .devcontainer && docker compose --profile linux run --rm linux bash scripts/build-linux.sh
# Output: dist/linux/ d256shell-<ver>-linux-x86_64.tar.gz (portable), d256shell_<ver>_amd64.deb, PKGBUILD for Arch,
# SHA256SUMS. The binary needs glibc >= 2.34 and WebKitGTK 4.1 only.
set -e
cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')
OUT=dist/linux
rm -rf "$OUT" && mkdir -p "$OUT"

(cd ui && npm ci --silent && npm run build 2>&1 | tail -2)
cargo build --release --locked -p d256shell --features custom-protocol 2>&1 | grep -E "^error|warning: unused|Finished"
BIN=${CARGO_TARGET_DIR:-target}/release/d256shell
strip "$BIN" 2>/dev/null || true

# ---- portable tar.gz: binary + empty data folder (its presence switches the program to portable mode)
P=$OUT/d256shell
mkdir -p "$P/data"
cp "$BIN" "$P/d256shell"
cp packaging/linux/d256shell.desktop packaging/linux/README-linux.txt LICENSE src-tauri/icons/128x128.png "$P/"
tar -C "$OUT" -czf "$OUT/d256shell-$VER-linux-x86_64.tar.gz" d256shell
rm -rf "$P"

# ---- .deb (Ubuntu / Debian)
D=$(mktemp -d)
install -Dm755 "$BIN" "$D/usr/bin/d256shell"
install -Dm644 packaging/linux/d256shell.desktop "$D/usr/share/applications/d256shell.desktop"
install -Dm644 src-tauri/icons/32x32.png "$D/usr/share/icons/hicolor/32x32/apps/d256shell.png"
install -Dm644 src-tauri/icons/128x128.png "$D/usr/share/icons/hicolor/128x128/apps/d256shell.png"
install -Dm644 "src-tauri/icons/128x128@2x.png" "$D/usr/share/icons/hicolor/256x256/apps/d256shell.png"
install -Dm644 LICENSE "$D/usr/share/doc/d256shell/copyright"
mkdir -p "$D/DEBIAN"
SIZE=$(du -ks "$D/usr" | cut -f1)
cat > "$D/DEBIAN/control" <<EOF
Package: d256shell
Version: $VER
Section: net
Priority: optional
Architecture: amd64
Depends: libwebkit2gtk-4.1-0, libgtk-3-0, openssh-client
Installed-Size: $SIZE
Maintainer: dest1256
Homepage: https://github.com/dest1256
Description: Portable SSH client with terminal, SFTP explorer and editor
 d256 Shell uses the system OpenSSH client for the terminal, keeps passwords in an
 encrypted vault, and offers an SFTP file explorer and a built-in text editor.
EOF
dpkg-deb --root-owner-group -Zxz -b "$D" "$OUT/d256shell_${VER}_amd64.deb" >/dev/null
rm -rf "$D"

# ---- Arch: PKGBUILD that installs the prebuilt tarball (checksum filled in)
SUM=$(sha256sum "$OUT/d256shell-$VER-linux-x86_64.tar.gz" | cut -d' ' -f1)
sed -e "s/@VER@/$VER/" -e "s/@SUM@/$SUM/" packaging/arch/PKGBUILD.in > "$OUT/PKGBUILD"
cp packaging/linux/d256shell.desktop LICENSE "$OUT/"

(cd "$OUT" && sha256sum d256shell-$VER-linux-x86_64.tar.gz d256shell_${VER}_amd64.deb > SHA256SUMS.txt && cat SHA256SUMS.txt)
ls -l "$OUT"
