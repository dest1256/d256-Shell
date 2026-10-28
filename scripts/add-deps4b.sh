#!/bin/bash
# Stage 4: opener plugin (open links in the system browser only after a click) + xterm npm packages.
cd "$(dirname "$0")/.."
cargo add -p d256shell tauri-plugin-opener
cargo info tauri-plugin-opener 2>/dev/null | grep -E "^(version|license|repository):"
cd ui
echo "== @tauri-apps/plugin-opener $(npm view @tauri-apps/plugin-opener version) $(npm view @tauri-apps/plugin-opener repository.url)"
npm install --save-exact @tauri-apps/plugin-opener @xterm/xterm@6.0.0 @xterm/addon-fit@0.11.0 @xterm/addon-webgl@0.19.0 @xterm/addon-unicode11@0.9.0 @xterm/addon-web-links@0.12.0 @xterm/addon-search@0.16.0 2>&1 | tail -4
grep -A12 '"dependencies"' package.json
