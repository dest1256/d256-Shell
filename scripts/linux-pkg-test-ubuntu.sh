#!/bin/bash
# In a clean Ubuntu container: install the .deb (dependencies come from apt) and run the smoke test as a normal user.
# usage: linux-pkg-test-ubuntu.sh <name>
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq /pkg/d256shell_*_amd64.deb xvfb xauth xdotool imagemagick dbus-x11 fonts-dejavu-core libgl1-mesa-dri >/dev/null
useradd -m -s /bin/bash u
mkdir -p /out && chmod 777 /out
runuser -u u -- bash /smoke.sh "$1"
