#!/bin/sh
# Test-only sshd. MODE: password | kbdint | 2fa. Credentials: see README.md (test data only).
set -e
# Fresh host keys per container (the package generates keys at image build time, which would make
# every container identical): sshd-otherkey must differ from sshd-password.
rm -f /etc/ssh/ssh_host_*
ssh-keygen -A
useradd -m -s /bin/bash tester 2>/dev/null || true
echo 'tester:testpass' | chpasswd
useradd -m -s /bin/bash otheruser 2>/dev/null || true
echo 'otheruser:otherpass' | chpasswd

H=/home/tester
mkdir -p "$H/zone/dir with spaces" "$H/zone/ro_dir" "$H/zone/empty" /protected
echo hi > "$H/zone/file with spaces.txt"; echo hi > "$H/zone/юникод-файл.txt"
echo hi > "$H/zone/-dash.txt"; echo hi > "$H/zone/it's \$weird *name.txt"
printf 'bad\n' > "$H/zone/$(printf 'bad\377name')"
printf 'a\r\nb\r\n' > "$H/zone/crlf.txt"
printf '\357\273\277bom\n' > "$H/zone/bom.txt"
printf 'no newline' > "$H/zone/nonl.txt"
head -c 4096 /dev/urandom > "$H/zone/binary.bin"
echo secret > /protected/a.txt; echo secret2 > /protected/b.txt
ln -sfn /protected "$H/zone/link_to_protected"
ln -sfn . "$H/zone/loop"
echo x > "$H/zone/other_owner.txt"; chown otheruser "$H/zone/other_owner.txt"
chown -R tester:tester "$H/zone/dir with spaces" "$H/zone/empty"
chmod 555 "$H/zone/ro_dir"; chown tester "$H/zone/ro_dir"
chown tester:tester "$H"/zone/*.txt "$H"/zone/*.bin 2>/dev/null || true
chown otheruser "$H/zone/other_owner.txt"
(cd /protected && sha256sum * > /protected.sha256)
# 10 000 files for listing/virtualization tests
mkdir -p "$H/zone/many" && (cd "$H/zone/many" && seq 1 10000 | sed 's/^/f/' | xargs touch) && chown -R tester:tester "$H/zone/many"

C=/etc/ssh/sshd_config.d/test.conf
{ echo "PubkeyAuthentication no"; echo "PermitRootLogin no"; echo "LogLevel VERBOSE"; echo "MaxAuthTries 6"; } > $C
case "$MODE" in
  nosftp)
    # like a server where SFTP is disabled: the SFTP subsystem is removed, the shell still works
    sed -i "/^Subsystem/d" /etc/ssh/sshd_config
    echo "PasswordAuthentication yes
KbdInteractiveAuthentication no
UsePAM no" >> $C ;;
  password) sed -i 's/PermitRootLogin no/PermitRootLogin yes/' $C; echo 'root:rootpass' | chpasswd   # test-only: lets tests check "su - user" without a password prompt
    echo "PasswordAuthentication yes
KbdInteractiveAuthentication no
UsePAM no" >> $C ;;
  key)
    # key login (test keys are generated at start into the shared /testkeys volume); the password stays allowed
    # so that "key rejected -> saved password" can be tested too
    sed -i 's/PubkeyAuthentication no/PubkeyAuthentication yes/' $C
    echo "PasswordAuthentication yes
KbdInteractiveAuthentication no
UsePAM no" >> $C
    mkdir -p /testkeys /home/tester/.ssh && rm -f /testkeys/id_*
    cd /testkeys
    ssh-keygen -q -t ed25519 -N '' -f id_ed25519 -C test-ed25519
    ssh-keygen -q -t rsa -b 2048 -N '' -f id_rsa -C test-rsa
    ssh-keygen -q -t rsa -b 2048 -m PEM -N '' -f id_rsa_pem -C test-rsa-pem
    ssh-keygen -q -t ecdsa -N '' -f id_ecdsa -C test-ecdsa
    ssh-keygen -q -t ed25519 -N 'keypass' -f id_enc -C test-encrypted
    ssh-keygen -q -t ed25519 -N '' -f id_other -C not-authorized
    printf 'PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n' > id_putty.ppk
    cat id_ed25519.pub id_rsa.pub id_rsa_pem.pub id_ecdsa.pub id_enc.pub > /home/tester/.ssh/authorized_keys
    chown -R tester:tester /home/tester/.ssh && chmod 700 /home/tester/.ssh && chmod 600 /home/tester/.ssh/authorized_keys
    # the dev container runs as uid 1000: ssh refuses keys that belong to somebody else or are readable by others
    chown -R 1000:1000 /testkeys && chmod 600 /testkeys/id_* && chmod 644 /testkeys/*.pub
    cd / ;;
  kbdint) echo "PasswordAuthentication no
KbdInteractiveAuthentication yes
UsePAM yes" >> $C ;;
  2fa)
    echo "PasswordAuthentication no
KbdInteractiveAuthentication yes
UsePAM yes
AuthenticationMethods keyboard-interactive" >> $C
    # password first, then TOTP ("Verification code:"); secret is test-only
    printf 'auth required pam_unix.so\nauth required pam_google_authenticator.so secret=/etc/ga_secret user=root\n@include common-account\n@include common-session\n' > /etc/pam.d/sshd
    printf 'JBSWY3DPEHPK3PXP\n" TOTP_AUTH\n' > /etc/ga_secret; chmod 600 /etc/ga_secret ;;   # pam_google_authenticator insists on 0400/0600
esac
# Log to a file on a shared volume so tests can count authentication attempts (e.g. "Failed password").
LOG="/var/log/sshtest/${LOG_NAME:-sshd}.log"
mkdir -p /var/log/sshtest && touch "$LOG" && chmod 644 "$LOG"
exec /usr/sbin/sshd -D -E "$LOG"
