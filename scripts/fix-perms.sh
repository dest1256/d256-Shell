#!/bin/bash
# Directories created from the Windows side appear as root:755 in the container.
# Run as root:  docker compose exec -u root dev bash scripts/fix-perms.sh
cd "$(dirname "$0")/.."
find . \( -path ./ui/node_modules -o -path ./e2e/node_modules -o -path ./.git \) -prune -o -type d ! -user dev -exec chown dev:dev {} +
echo "perms fixed"
