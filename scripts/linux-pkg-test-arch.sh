#!/bin/bash
# In a clean Arch container: build the package from PKGBUILD with makepkg, install it with pacman, smoke test as a normal user.
set -e
pacman -Syu --noconfirm --needed base-devel webkit2gtk-4.1 gtk3 openssh xorg-server-xvfb xorg-xauth xdotool imagemagick ttf-dejavu mesa >/dev/null
useradd -m -s /bin/bash u
mkdir -p /out /home/u/build && chmod 777 /out
cp /pkg/PKGBUILD /pkg/d256shell-*-linux-x86_64.tar.gz /pkg/d256shell.desktop /pkg/COPYRIGHT /home/u/build/
chown -R u:u /home/u/build
runuser -u u -- bash -c 'cd /home/u/build && makepkg -f --noconfirm 2>&1 | tail -4'
pacman -U --noconfirm /home/u/build/d256shell-*.pkg.tar.zst >/dev/null
pacman -Qi d256shell | grep -E "^(Name|Version|Depends On)"
runuser -u u -- bash /smoke.sh "$1"
