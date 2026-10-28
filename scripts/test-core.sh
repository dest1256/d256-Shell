#!/bin/bash
# Run core unit tests (dev container). Optional args: cargo test filters/flags. Prints a summary and any failure details.
cd "$(dirname "$0")/.."
out=$(cargo test -p d256shell-core --lib "$@" 2>&1)
rc=$?
echo "$out" | grep -E "^test result|^error|^warning: unused"
echo "$out" | grep -E "FAILED|^---- |panicked|^  left|^ right|^  right|assertion" | head -60
exit $rc
