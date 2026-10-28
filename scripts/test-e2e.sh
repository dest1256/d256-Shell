#!/bin/bash
# Build UI + devbridge, then run Playwright e2e (dev container).
set -e
cd "$(dirname "$0")/.."
(cd ui && npm ci --silent && npm run build)
cargo build --locked -p d256shell-devbridge
cd e2e
[ -d node_modules ] || npm install --silent
ls "$PLAYWRIGHT_BROWSERS_PATH"/chromium-* >/dev/null 2>&1 || npx playwright install chromium
npx playwright test "$@" 2>&1 | tail -60
