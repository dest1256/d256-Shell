# Как выпустить обновление

Один раз: репозиторий `https://github.com/dest1256/d256-Shell` уже подключён как `origin` в папке `github`.
Дальше для каждого обновления:

1. **Поднять версию** (правила в [VERSIONING.md](VERSIONING.md)):
   `bash scripts/bump-version.sh patch` (исправления) · `minor` (новые функции) · `major` (несовместимые изменения).
   Скрипт меняет номер везде и добавляет заголовок в `CHANGELOG.md`.
2. **Описать изменения** в `CHANGELOG.md` под новым заголовком (Добавлено / Изменено / Исправлено).
3. **Проверить и собрать** (в dev-контейнере, из `.devcontainer`):
   ```
   docker compose exec dev bash scripts/check-all.sh
   docker compose exec dev bash scripts/make-dist.sh
   docker compose --profile linux run --rm linux bash scripts/build-linux.sh
   ```
4. **Собрать папку для GitHub:** `bash scripts/sync-github.sh` — обновит `github/` (исходники, документы) и `github/releases/` (сборки).
5. **Закоммитить и поставить тег** (в папке `github`, новый коммит, историю не переписывать, `--amend` не использовать):
   ```
   cd github
   git add -A
   git commit -m "d256 Shell X.Y.Z: коротко что изменилось"
   git tag -a vX.Y.Z -m "d256 Shell X.Y.Z"
   git push origin main --tags
   ```
6. **Создать релиз на GitHub:** репозиторий → Releases → Draft a new release → Tag `vX.Y.Z` → Title `d256 Shell X.Y.Z` →
   описание из `CHANGELOG.md` → прикрепить **все** файлы из `github/releases/` → Publish release.
   (Если включить `.github/workflows/release.yml`, пакеты соберёт и прикрепит GitHub Actions по тегу `v*`.)

Правила: тег и версия в `Cargo.toml` всегда совпадают; опубликованный тег не переносить и коммиты, уже отправленные на GitHub, не переписывать.
