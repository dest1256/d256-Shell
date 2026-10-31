# How to publish an update

Once: the repository `https://github.com/dest1256/d256-Shell` is already connected as `origin` in the `github` folder.
Then for every update:

1. **Raise the version** (rules in [VERSIONING.md](VERSIONING.md)):
   `bash scripts/bump-version.sh patch` (fixes) · `minor` (new features) · `major` (incompatible changes).
   The script changes the number everywhere and adds a heading to `CHANGELOG.md`.
2. **Describe the changes** in `CHANGELOG.md` under the new heading (Added / Changed / Fixed).
3. **Check and build** (in the dev container, from `.devcontainer`):
   ```
   docker compose exec dev bash scripts/check-all.sh
   docker compose exec dev bash scripts/make-dist.sh
   docker compose --profile linux run --rm linux bash scripts/build-linux.sh
   ```
4. **Prepare the GitHub folder:** `bash scripts/sync-github.sh` updates `github/` (sources, documents) and `github/releases/` (builds).
5. **Commit and tag** (in the `github` folder, a new commit, never rewrite history, do not use `--amend`):
   ```
   cd github
   git add -A
   git commit -m "d256 Shell X.Y.Z: short summary of the change"
   git tag -a vX.Y.Z -m "d256 Shell X.Y.Z"
   git push origin main --tags
   ```
6. **Create the release on GitHub:** repository → Releases → Draft a new release → Tag `vX.Y.Z` → Title `d256 Shell X.Y.Z` →
   description from `CHANGELOG.md` → attach **all** files from `github/releases/` → Publish release.
   (If `.github/workflows/release.yml` is enabled, GitHub Actions builds and attaches the packages for a `v*` tag.)

Rules: the tag and the version in `Cargo.toml` always match; never move a published tag and never rewrite commits already pushed to GitHub.
