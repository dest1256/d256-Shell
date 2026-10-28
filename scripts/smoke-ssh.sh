#!/bin/bash
# Smoke test: password login against the test sshd containers (dev container only).
printf '#!/bin/sh\necho "${SMOKE_PW:-testpass}"\n' > /tmp/ap && chmod +x /tmp/ap
try() { # host user pw
  SMOKE_PW="$3" SSH_ASKPASS=/tmp/ap SSH_ASKPASS_REQUIRE=force ssh -F none \
    -o UserKnownHostsFile=/tmp/kh_$1 -o StrictHostKeyChecking=accept-new -o PubkeyAuthentication=no \
    -o PreferredAuthentications=keyboard-interactive,password -o NumberOfPasswordPrompts=1 \
    "$2@$1" "echo marker-ok; id -un" 2>&1 | tr '\n' ' '; echo
}
for h in sshd-password sshd-kbdint sshd-otherkey; do echo -n "$h: "; try $h tester testpass; done
echo -n "wrong pw (expect denied): "; try sshd-password tester wrong
echo -n "sshd-2fa (expect denied w/o code): "; try sshd-2fa tester testpass
