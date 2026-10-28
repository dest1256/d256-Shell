#!/bin/bash
# Facts about the resolved tauri crate: window builder methods and window permissions.
R=/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f
T=$(ls -d $R/tauri-2.*/ | tail -1)
echo "tauri: $T"
grep -rn "pub fn decorations\|pub fn shadow\|pub fn resizable" "$T/src/webview/webview_window.rs" | head
echo "--- window permissions"
grep -rhoE "allow-(start-dragging|toggle-maximize|minimize|close|is-maximized|unmaximize|maximize|start-resize-dragging)\b" "$T/permissions/" | sort -u
