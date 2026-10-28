#!/bin/bash
# Assemble the `github/` folder: everything that goes into the GitHub repository (sources, licences, docs, workflows)
# and, separately in github/releases/, the finished builds to attach to a GitHub Release (not committed: .gitignore).
# Run after the builds: scripts/make-dist.sh (Windows) and scripts/build-linux.sh (Linux).
set -e
cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')
G=github
mkdir -p "$G"
# sources are replaced as a whole (so renamed / deleted files do not linger); the GitHub-only files
# (README.md, .gitignore, docs/, .github/ISSUE_TEMPLATE) stay
rm -f "$G"/LICENSE "$G"/NOTICE   # no licence file any more (private repository, see COPYRIGHT)
rm -rf "$G"/crates "$G"/src-tauri "$G"/ui "$G"/e2e "$G"/test-server "$G"/packaging "$G"/scripts "$G"/.devcontainer "$G"/.cargo "$G"/.github/workflows "$G"/releases
tar cf - --exclude=node_modules --exclude=./ui/dist --exclude=target --exclude=./dist --exclude=./github \
  --exclude=test-results --exclude=./e2e/screenshots --exclude=./PROMPT.md --exclude=./src-tauri/gen \
  --exclude=./README.md --exclude=./.gitignore --exclude=./.git . | tar xf - -C "$G"
# the README of the repository mentions the package file names: keep them at the current version
perl -pi -e "s/(d256shell[_-])\d+\.\d+\.\d+([_-](?:amd64|linux))/\${1}$VER\$2/g" "$G/README.md"
# builds for the Release page
mkdir -p "$G/releases"
[ -f dist/d256Shell.zip ] && cp dist/d256Shell.zip "$G/releases/" && cp dist/SHA256SUMS.txt "$G/releases/SHA256SUMS-windows.txt"
if [ -d dist/linux ]; then
  cp dist/linux/d256shell-$VER-linux-x86_64.tar.gz dist/linux/d256shell_${VER}_amd64.deb dist/linux/PKGBUILD "$G/releases/"
  cp dist/linux/SHA256SUMS.txt "$G/releases/SHA256SUMS-linux.txt"
fi
echo "github/ is ready for version $VER:"; ls "$G" "$G/releases"
