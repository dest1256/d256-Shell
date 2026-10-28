d256 Shell for Linux (x86_64)
============================
Needs: WebKitGTK 4.1 (libwebkit2gtk-4.1), GTK 3 and the OpenSSH client (ssh).

  Ubuntu / Debian:  sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 openssh-client
  Arch Linux:       sudo pacman -S webkit2gtk-4.1 gtk3 openssh

Portable use: run ./d256shell from this folder. Its data (vault, known_hosts, settings) stays in the "data"
folder next to the program: copy the whole folder to move it.

Installed from a package (deb / PKGBUILD): the program lives in /usr/bin and keeps its data in
~/.local/share/d256shell (or $XDG_DATA_HOME/d256shell). To use another folder: D256SHELL_DATA_DIR=/path d256shell

Copyright (c) 2026 dest1256. All rights reserved (private software, see COPYRIGHT).
