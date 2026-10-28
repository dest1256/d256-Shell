#!/bin/bash
# Inspect embedded resources of the built Windows exe (run in dev container).
exe="${1:-dist/d256Shell/d256Shell.exe}"
echo "== resources"; llvm-readobj --coff-resources "$exe" | grep -E "Type|Name" | sort | uniq -c | head -20
echo "== version info / authorship"; strings -el "$exe" | grep -E "dest1256|d256Shell" | sort -u | head
echo "== manifest"; strings "$exe" | grep -E "asInvoker|PerMonitor|longPathAware|supportedOS" | sort -u | head
