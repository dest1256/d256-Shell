# Тестовые SSH-серверы (только для тестов!)

| Порт (127.0.0.1) | Сервис          | Режим |
|------|-----------------|-------|
| 2221 | sshd-password   | только password |
| 2222 | sshd-kbdint     | только keyboard-interactive (`Password:`) |
| 2223 | sshd-2fa        | пароль + `Verification code:` (TOTP, секрет `JBSWY3DPEHPK3PXP`) |
| 2224 | sshd-otherkey   | как 2221, но другой host key (тест «ключ изменился») |
| 2225 | sshd-nosftp     | как 2221, но SFTP-подсистема отключена (оболочка работает) |

Пользователь `tester` / пароль `testpass`; `otheruser` / `otherpass` (владелец файла `other_owner.txt`).
Тестовая зона: `/home/tester/zone`; эталон для проверки удаления: `/protected` (+ `/protected.sha256`).
Host keys генерируются при старте контейнера и меняются при пересоздании.
Не подключайте эти серверы никуда, кроме тестов. Заглушка без posix-rename — будет добавлена на этапе 5.
