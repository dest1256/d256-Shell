#!/bin/bash
# Stage 4 dependencies: portable-pty (Rust) and @xterm/* (npm). Print metadata to verify the official source.
cd "$(dirname "$0")/.."
cargo add -p d256shell-core portable-pty
cargo info portable-pty 2>/dev/null | grep -E "^(version|license|repository):"
cd ui
for p in @xterm/xterm @xterm/addon-fit @xterm/addon-webgl @xterm/addon-unicode11 @xterm/addon-web-links @xterm/addon-search; do
  echo "== $p $(npm view $p version 2>/dev/null) | $(npm view $p repository.url 2>/dev/null) | $(npm view $p license 2>/dev/null) | maintainers: $(npm view $p maintainers --json 2>/dev/null | tr -d '\n ' | head -c 150)"
done
