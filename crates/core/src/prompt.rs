//! Classification of authentication prompts (shared by SFTP keyboard-interactive and the ssh askpass helper).

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PromptKind {
    /// Ordinary account password request: the saved password may be supplied (once).
    Password,
    /// Passphrase of a private key file: masked input, never the account password.
    Passphrase,
    /// Anything else (key passphrase, OTP / verification code, yes/no, unknown text): ask the user.
    Other,
}

/// Substrings that mean "this is not the account password".
const NOT_PASSWORD: [&str; 10] = [
    "verification",
    "code",
    "otp",
    "token",
    "yes/no",
    "fingerprint",
    "new password",
    "retype",
    "новый пароль",
    "одноразов",
];

/// Is `text` a request for the account password (and not a key passphrase or one-time code)?
/// Unknown text is `Other`: when in doubt the user is asked instead of sending the saved password.
pub fn classify(text: &str) -> PromptKind {
    let lower = text.trim().to_lowercase();
    let t = lower.trim_end_matches([' ', ':', '\u{a0}']);
    if t.contains("passphrase") {
        return PromptKind::Other;
    }
    // OpenSSH form "user@host's password": user/host may contain any word, so decide before the word lists.
    if t.ends_with("'s password") {
        return PromptKind::Password;
    }
    let has_word = |w: &str| t.split(|c: char| !c.is_alphanumeric()).any(|x| x == w);
    if NOT_PASSWORD.iter().any(|w| t.contains(w)) || has_word("pin") {
        return PromptKind::Other;
    }
    if t == "password"
        || t.starts_with("password for ")
        || t == "пароль"
        || t.ends_with(" password")
    {
        PromptKind::Password
    } else {
        PromptKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_prompts() {
        for p in [
            "Password:",
            "password: ",
            "root@host's password:",
            "tester@sshd-password's password: ",
            "dev@vscode-code-host's password:",
            "Password for tester@host:",
            "PASSWORD:",
            "Пароль:",
            "Shipping password:",
        ] {
            assert_eq!(classify(p), PromptKind::Password, "{p:?}");
        }
    }

    #[test]
    fn not_password_prompts() {
        for p in [
            "Enter passphrase for key '/home/u/.ssh/id_ed25519':",
            "Verification code:",
            "Enter passphrase for /x:",
            "One-time password (OTP):",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])?",
            "Token:",
            "Enter PIN for 'key':",
            "New password:",
            "Retype new password:",
            "Please enter your authentication code:",
            "Password (code):",
            "",
            "Username:",
        ] {
            assert_eq!(classify(p), PromptKind::Other, "{p:?}");
        }
    }
}
