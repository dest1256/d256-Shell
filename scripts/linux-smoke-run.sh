#!/bin/bash
# Runs INSIDE a clean distro container as an ordinary user, after the package is installed:
# starts the installed program on a virtual display, checks the window, the XDG data folder, and takes a screenshot.
# usage: linux-smoke-run.sh <screenshot-name>
set -e
NAME=${1:-smoke}
export WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 LIBGL_ALWAYS_SOFTWARE=1
command -v d256shell
d256shell --version >/dev/null 2>&1 || true
xvfb-run -a -s "-screen 0 1280x800x24" bash -c '
  d256shell >/tmp/app.log 2>&1 &
  PID=$!
  for i in $(seq 1 40); do
    WID=$(xdotool search --name "d256 Shell" 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.5
  done
  [ -n "$WID" ] || { echo "NO WINDOW"; cat /tmp/app.log; exit 1; }
  sleep 4
  import -window root /out/'"$NAME"'.png
  kill $PID 2>/dev/null || true
  wait $PID 2>/dev/null || true
'
D=${XDG_DATA_HOME:-$HOME/.local/share}/d256shell
[ -d "$D/webview" ] && [ -d "$D/logs" ] || { echo "DATA FOLDER MISSING: $D"; exit 1; }
echo "OK $NAME: window shown, data in $D"
