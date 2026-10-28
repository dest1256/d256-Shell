#!/bin/bash
# Human readable cargo audit report (vulnerabilities first).
cd "$(dirname "$0")/.."
cargo audit 2>&1 | grep -v "Fetching\|Loaded\|Scanning\|Updating" | head -${LINES_MAX:-80}
