# Зависимости d256 Shell

Автор и правообладатель: **dest1256**. Все пакеты — только с crates.io и registry.npmjs.org, без git-зависимостей,
форков и сторонних зеркал. `Cargo.lock` и `package-lock.json` закоммичены; сборка — `cargo build --locked`, `npm ci`.
У npm включено `ignore-scripts=true` (`ui/.npmrc`, `e2e/.npmrc`): ни один пакет не запускает install-скрипты.
Версии прямых npm-зависимостей зафиксированы точно (без `^`/`~`).

Перед добавлением каждого пакета проверялось: точное имя (защита от тайпсквоттинга), репозиторий в метаданных, активность,
отсутствие свежего подозрительного релиза (для npm и `tauri-plugin-dialog` выбраны версии старше двух недель на момент выбора).

## Rust: прямые зависимости (попадают в exe)

| Крейт | Версия | Назначение | Репозиторий | Лицензия | Почему нужен |
|---|---|---|---|---|---|
| tauri | 2.12.5 | Окно, WebView2, IPC | github.com/tauri-apps/tauri | Apache-2.0 OR MIT | Оболочка приложения (по ТЗ) |
| tauri-build | 2.7.4 (build) | Сборочный скрипт Tauri, ресурсы exe, манифест | github.com/tauri-apps/tauri | Apache-2.0 OR MIT | Версия/манифест/иконка exe |
| tauri-plugin-opener | 2.7.2 | Открытие ссылок из терминала в системном браузере | github.com/tauri-apps/plugins-workspace | Apache-2.0 OR MIT | Ссылки только по клику, без открытия внутри WebView |
| tauri-plugin-dialog | =2.8.1 | Системное окно выбора файлов/папок | github.com/tauri-apps/plugins-workspace | Apache-2.0 OR MIT | «Загрузить файлы», «Скачать…» (нужны настоящие локальные пути) |
| drag | 2.1.1 | Нативное перетаскивание файлов из панели в Проводник Windows | github.com/crabnebula-dev/drag-rs | Apache-2.0 OR MIT | Drag-out (по ТЗ, п. 6.4) |
| russh | 0.65.0 | SSH-клиент для SFTP | github.com/warp-tech/russh | Apache-2.0 | SFTP-соединение, проверка ключа хоста |
| russh-sftp | 3.0.1 | SFTP поверх russh | github.com/AspectUnk/russh-sftp | Apache-2.0 | Операции проводника |
| portable-pty | 0.9.0 | Псевдоконсоль (ConPTY на Windows, PTY на Linux) | github.com/wezterm/wezterm | MIT | Запуск системного ssh в терминале |
| tokio | 1.54.1 | Асинхронный рантайм | github.com/tokio-rs/tokio | MIT | SFTP, очередь передач |
| argon2 | 0.6.0 | KDF мастер-пароля (Argon2id) | github.com/RustCrypto/password-hashes | MIT OR Apache-2.0 | Хранилище паролей |
| chacha20poly1305 | 0.11.0 | Шифрование хранилища (XChaCha20-Poly1305) | github.com/RustCrypto/AEADs | Apache-2.0 OR MIT | Хранилище паролей |
| encoding_rs | =0.8.42 | Кодировки текста в редакторе (Windows-1251, KOI8-R, UTF-16, Shift_JIS, GBK …) | github.com/hsivonen/encoding_rs | (Apache-2.0 OR MIT) AND BSD-3-Clause | Открытие и сохранение файлов не только в UTF-8 (реализация стандарта WHATWG Encoding, тот же код, что в Firefox) |
| chardetng | =1.0.0 | Автоопределение кодировки файла | github.com/hsivonen/chardetng | Apache-2.0 OR MIT | Файл в Windows-1251/KOI8-R открывается без вопросов; тот же автор и стандарт, что у encoding_rs |
| zeroize | 1.9.1 | Затирание секретов в памяти | github.com/RustCrypto/utils | Apache-2.0 OR MIT | Пароли и ключи |
| getrandom | 0.4.3 | ОС-генератор случайных чисел | github.com/rust-random/getrandom | MIT OR Apache-2.0 | Соли, nonce, токен askpass, имена временных файлов |
| hmac, sha1 | 0.13.0, 0.11.0 | Сверка хэшированных записей `known_hosts` (HMAC-SHA1) | github.com/RustCrypto/MACs, /hashes | MIT OR Apache-2.0 | Формат known_hosts OpenSSH |
| sha2 | 0.11.1 | SHA-256 версии файла в редакторе | github.com/RustCrypto/hashes | MIT OR Apache-2.0 | Обнаружение изменения файла на сервере |
| base64ct | 1.8.3 | Base64 (known_hosts, IPC терминала) | github.com/RustCrypto/formats | Apache-2.0 OR MIT | Форматы OpenSSH, ввод терминала |
| serde, serde_json | 1.0.229, 1.0.154 | Сериализация API и настроек | github.com/serde-rs | MIT OR Apache-2.0 | JSON-API, settings.json |

Только для тестов (`dev-dependencies` devbridge): base64ct, getrandom, hmac, sha1 — генерация кода TOTP для теста 2FA.

Крейт `devbridge` (HTTP-мост для e2e-тестов) в Windows-exe не входит.

## npm: прямые зависимости интерфейса (`ui/`, попадают в exe)

| Пакет | Версия | Назначение | Репозиторий | Лицензия |
|---|---|---|---|---|
| @xterm/xterm | 6.0.0 | Терминал | github.com/xtermjs/xterm.js | MIT |
| @xterm/addon-fit, -search, -serialize, -unicode11, -web-links, -webgl | 0.11.0, 0.16.0, 0.14.0, 0.9.0, 0.12.0, 0.19.0 | Размер, поиск, снимок экрана (дублирование терминала), Unicode 11, ссылки, WebGL | github.com/xtermjs/xterm.js | MIT |
| @tauri-apps/api | 2.12.4 | IPC Tauri, окно, drag-drop событие | github.com/tauri-apps/tauri | Apache-2.0 OR MIT |
| @tauri-apps/plugin-opener | 2.7.2 | Открытие ссылок | github.com/tauri-apps/plugins-workspace | MIT OR Apache-2.0 |
| @tauri-apps/plugin-dialog | 2.8.1 | Выбор файлов | github.com/tauri-apps/plugins-workspace | MIT OR Apache-2.0 |
| @codemirror/state, view, commands, language, search | 6.7.6, 6.43.13, 6.11.1, 6.12.4, 6.7.2 | Редактор (ядро, ввод, история, подсветка, поиск/замена) | code.haverbeke.berlin/codemirror (официальный репозиторий автора CodeMirror; зеркало github.com/codemirror) | MIT |
| @codemirror/merge | 6.12.2 | Показ различий при конфликте сохранения | то же | MIT |
| @codemirror/lang-javascript, -json, -yaml, -xml, -html, -css, -python, -php, -sql, -markdown | см. `ui/package.json` | Подсветка синтаксиса | github.com/codemirror | MIT |
| @codemirror/legacy-modes | 6.5.4 | sh/bash, nginx, toml, properties (ini/conf/systemd), Dockerfile | code.haverbeke.berlin/codemirror | MIT |
| @lezer/highlight | 1.2.5 | Теги подсветки для темы редактора | code.haverbeke.berlin/lezer | MIT |

Dev-зависимости сборки (в exe не попадают): `typescript` 7.0.2, `vite` 8.3.5, `@playwright/test` 1.64.1 (`e2e/`, только тесты).

Примечание: репозитории `@codemirror/*` перенесены автором (Marijn Haverbeke) на собственный сервер `code.haverbeke.berlin`;
в метаданных пакетов на npm это значится в `repository.url`, владелец пакетов на npm прежний.

## Инструменты (только в dev-контейнере)

`cargo-xwin`, `tauri-cli`, `cargo-audit`, `cargo-deny`, `cargo-about` — `cargo install --locked` с crates.io; образы `rust`, `node`, `debian`
закреплены по дайджесту (`.devcontainer/Dockerfile`). MSVC CRT/SDK — через `cargo-xwin` с серверов Microsoft.
В Windows ничего не устанавливается. `ssh.exe` в приложение не встроен: используется системный OpenSSH Windows.

## Проверки на момент сборки

Запускаются `scripts/check-all.sh`:

- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok (граф только для Windows-цели). Исключение: RUSTSEC-2023-0071 (`rsa`,
  Marvin attack, исправления нет) — тянется `russh`; приватных RSA-операций в d256 Shell нет (вход только по паролю, ключи хостов лишь проверяются),
  обоснование в `deny.toml` и `.cargo/audit.toml`.
- `cargo audit`: 0 уязвимостей (то же исключение).
- `npm audit --audit-level=moderate`: `ui/` (production) — 0 уязвимостей; `e2e/` — 0 уязвимостей.
- Лицензии зависимостей — только разрешающие (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 …); копилефта нет.
  Полные тексты лицензий встроены в exe (окно «О программе» → «Лицензии библиотек»), генерируются `scripts/gen-licenses.mjs`.

## Сеть

Приложение не делает сетевых запросов, кроме SSH к серверам пользователя: нет обновлений, телеметрии и отчётов о сбоях.
Фронтенд полностью встроен (никаких CDN и удалённых шрифтов), CSP `default-src 'self'`.
