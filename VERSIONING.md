# Versions

Version number `MAJOR.MINOR.PATCH` (semantic versioning, https://semver.org/):

| What changed | Which part to raise | Example |
|---|---|---|
| Bugs fixed, nothing new | **PATCH** | 1.0.0 → 1.0.1 |
| Features added, everything old still works | **MINOR** (patch is reset) | 1.0.1 → 1.1.0 |
| Incompatible change: old data cannot be opened without conversion, a feature is removed, behavior that people relied on changed | **MAJOR** (minor and patch are reset) | 1.4.2 → 2.0.0 |

Examples for this project:
- PATCH: a layout fix, a fix for a crash when saving a file, a dependency update without new features.
- MINOR: a new tab, a new encoding, a new setting, a new package type.
- MAJOR: a new password vault format (`vault.bin`) incompatible with the old one; removing a feature; changing the data folder without an automatic migration.

How to raise the version: `scripts/bump-version.sh patch|minor|major` (or `X.Y.Z`). The script changes the number everywhere at once
(`Cargo.toml` is the single source, plus Tauri, npm, the Windows manifest, lock files, package names in the README) and adds a
heading to `CHANGELOG.md`. Then: describe the changes in the changelog, build the packages, commit and tag `v<version>`
(a `v*` tag starts the release build in GitHub Actions). The version is shown in Settings → About and in the exe properties.
