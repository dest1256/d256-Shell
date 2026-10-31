# Test SSH servers (for tests only!)

| Port (127.0.0.1) | Service         | Mode |
|------|-----------------|-------|
| 2221 | sshd-password   | password only |
| 2222 | sshd-kbdint     | keyboard-interactive only (`Password:`) |
| 2223 | sshd-2fa        | password + `Verification code:` (TOTP, secret `JBSWY3DPEHPK3PXP`) |
| 2224 | sshd-otherkey   | like 2221 but with a different host key (the "key changed" test) |
| 2225 | sshd-nosftp     | like 2221 but the SFTP subsystem is disabled (the shell works) |

User `tester` / password `testpass`; `otheruser` / `otherpass` (owner of the file `other_owner.txt`).
Test zone: `/home/tester/zone`; reference for the deletion check: `/protected` (+ `/protected.sha256`).
Host keys are generated when the container starts and change when it is recreated.
Do not connect these servers to anything except the tests. A stub without posix-rename will be added in stage 5.
