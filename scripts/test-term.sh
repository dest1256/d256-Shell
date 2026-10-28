#!/bin/bash
# Terminal integration tests (real ssh in a PTY against the docker sshd containers). Dev container only.
cd "$(dirname "$0")/.."
out=$(cargo test -p d256shell-devbridge --test terminal "$@" -- --test-threads=1 --nocapture 2>&1)
rc=$?
echo "$out" | grep -E "^test |^test result|^error|^warning: unused|50 MB output" | head -60
echo "$out" | grep -E "panicked|^---- |timeout|terminal exited|Output:" -A6 | head -80
exit $rc
