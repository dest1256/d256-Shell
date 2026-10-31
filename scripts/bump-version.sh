#!/bin/bash
# Raise the version everywhere at once.
#   scripts/bump-version.sh patch    1.0.0 -> 1.0.1   bug fixes only
#   scripts/bump-version.sh minor    1.0.0 -> 1.1.0   new features, nothing breaks
#   scripts/bump-version.sh major    1.0.0 -> 2.0.0   incompatible change (see VERSIONING.md)
#   scripts/bump-version.sh 1.4.2                     an explicit version
# Updates Cargo.toml (the single source of truth), the Tauri / npm / Windows-manifest copies, the lock files,
# the package file names in README.md and adds a heading to CHANGELOG.md (fill it in, then commit and tag v<version>).
set -e
cd "$(dirname "$0")/.."
cur=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')
IFS=. read -r MA MI PA <<< "$cur"
case "${1:-}" in
  major) new="$((MA + 1)).0.0" ;;
  minor) new="$MA.$((MI + 1)).0" ;;
  patch) new="$MA.$MI.$((PA + 1))" ;;
  [0-9]*.[0-9]*.[0-9]*) new="$1" ;;
  *) echo "usage: $0 major|minor|patch|X.Y.Z   (current: $cur)"; exit 1 ;;
esac
echo "$cur -> $new"

perl -0pi -e "s/^version = \"\Q$cur\E\"/version = \"$new\"/m" Cargo.toml
for f in ui/package.json e2e/package.json src-tauri/tauri.conf.json; do
  perl -0pi -e "s/(\"version\": )\"\Q$cur\E\"/\$1\"$new\"/" "$f"
done
perl -pi -e "s/\Q$cur\E\.0/$new.0/" src-tauri/d256shell.manifest
perl -0pi -e "s/(name = \"d256shell(?:-core|-devbridge)?\"\nversion = )\"\Q$cur\E\"/\$1\"$new\"/g" Cargo.lock
perl -0pi -e "s/(\"name\": \"d256shell-(?:ui|e2e)\",\n\s*\"version\": )\"\Q$cur\E\"/\$1\"$new\"/g" ui/package-lock.json e2e/package-lock.json
perl -pi -e "s/(d256shell[_-])\d+\.\d+\.\d+([_-](?:amd64|linux))/\${1}$new\$2/g" README.md
# CHANGELOG: new heading on top (newest first)
if ! grep -q "^## $new " CHANGELOG.md 2>/dev/null; then
  d=$(date +%F)
  perl -CSDA -Mutf8 -0pi -e "s~(# Changelog\n)~\$1\n## $new — $d\n- (describe the changes: Added, Changed, Fixed)\n~" CHANGELOG.md
fi
echo "Next: describe the changes in CHANGELOG.md, then build (make-dist.sh, build-linux.sh), commit and tag v$new."
