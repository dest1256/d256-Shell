#!/bin/bash
# Full local gate (dev container): lint, unit tests, integration tests, e2e, audit.
cd "$(dirname "$0")/.."
rc=0
noise='^ *Compiling\|^ *Checking\|^ *Downloaded\|^ *Downloading'

echo "== lint"
bash scripts/clippy.sh | tail -3 ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1

echo "== unit tests"
cargo test --locked -p d256shell-core --lib 2>&1 | grep -E "^test result|FAILED|panicked" ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1

echo "== integration tests (docker sshd, single thread: they count sshd log lines)"
out=$(cargo test --locked -p d256shell-core --test '*' -- --test-threads=1 2>&1)
echo "$out" | grep -E "^test result|FAILED|panicked"
echo "$out" | grep -q "SKIPPED" && { echo "ERROR: integration tests were skipped"; rc=1; }
echo "$out" | grep -q "test result: FAILED" && rc=1

echo "== terminal tests (real ssh in a PTY; this package's binary is the askpass helper)"
tout=$(cargo test --locked -p d256shell-devbridge --test terminal -- --test-threads=1 2>&1)
echo "$tout" | grep -E "^test result|FAILED|panicked"
echo "$tout" | grep -q "SKIPPED" && { echo "ERROR: terminal tests were skipped"; rc=1; }
echo "$tout" | grep -q "test result: FAILED" && rc=1

echo "== e2e"
eout=$(bash scripts/test-e2e.sh 2>&1)
echo "$eout" | grep -E "passed|failed|✘|Error"
echo "$eout" | grep -qE "^ +[0-9]+ (failed|flaky)|✘|Error:" && rc=1
echo "$eout" | grep -qE "^ +[0-9]+ passed" || rc=1

echo "== audit / supply chain"
cargo audit 2>&1 | tail -3 ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1
cargo deny check 2>&1 | grep -E "^error|advisories|bans|licenses|sources" | tail -5 ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1
(cd ui && npm audit --omit=dev --audit-level=moderate 2>&1 | tail -2) ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1
(cd e2e && npm audit --audit-level=moderate 2>&1 | tail -2) ; [ "${PIPESTATUS[0]}" = 0 ] || rc=1
echo "check-all rc=$rc"
exit $rc
