//! Building the `ssh` argument vector. Every argument is a separate element (no string concatenation),
//! the security-relevant options come FIRST (OpenSSH keeps the first value of an option, so later
//! user-supplied duplicates cannot override them), and user extras are validated against an allowlist.
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct ArgsError(pub String);

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ArgsError {}

fn err<T>(m: impl Into<String>) -> Result<T, ArgsError> {
    Err(ArgsError(m.into()))
}

pub struct SshTarget<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    /// Free-form extra ssh arguments from the server settings.
    pub extra: &'a str,
    /// Private key file for key login (`-i`); None = no key, password / keyboard-interactive only.
    pub identity: Option<&'a Path>,
}

/// Value of `-o UserKnownHostsFile=...`: forward slashes (safe on Windows too) and double quotes
/// when the path contains whitespace, because OpenSSH splits option values on whitespace.
fn known_hosts_value(p: &Path) -> Result<String, ArgsError> {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.contains(['"', '\n', '\r', '\0']) {
        return err("Путь к known_hosts содержит недопустимые символы.");
    }
    Ok(if s.contains(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s
    })
}

/// Split on whitespace, honouring single and double quotes (no escapes).
pub fn split_extra(s: &str) -> Result<Vec<String>, ArgsError> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if quote.is_some() {
        return err("Незакрытая кавычка в дополнительных аргументах ssh.");
    }
    if has {
        out.push(cur);
    }
    Ok(out)
}

/// `-o` option names that would weaken host key verification or run local commands without being an
/// explicit proxy. Compared case-insensitively.
const FORBIDDEN_OPTIONS: [&str; 16] = [
    "stricthostkeychecking",
    "userknownhostsfile",
    "globalknownhostsfile",
    "updatehostkeys",
    "hostkeyalias",
    "verifyhostkeydns",
    "checkhostip",
    "knownhostscommand",
    "include",
    "localcommand",
    "permitlocalcommand",
    "remotecommand",
    "requesttty",
    "batchmode",
    "preferredauthentications",
    "pubkeyauthentication",
];

/// Flags without an argument that are safe for an interactive terminal session.
const FLAGS: [char; 13] = [
    '4', '6', 'A', 'a', 'C', 'g', 'K', 'k', 'q', 'v', 'X', 'x', 'Y',
];
/// Flags that take an argument.
const ARG_FLAGS: [char; 7] = ['L', 'R', 'D', 'J', 'i', 'c', 'm'];

fn check_option(spec: &str) -> Result<(), ArgsError> {
    let key = spec
        .split(|c: char| c == '=' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if key.is_empty() {
        return err("Пустая опция -o.");
    }
    if FORBIDDEN_OPTIONS.contains(&key.as_str()) {
        return err(format!(
            "Опция «{key}» запрещена: она ослабляет проверку ключа сервера или меняет поведение терминала."
        ));
    }
    Ok(())
}

/// Validate user extras and return them as argv elements.
pub fn validate_extra(extra: &str) -> Result<Vec<String>, ArgsError> {
    let toks = split_extra(extra)?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if t.contains(['\0', '\n', '\r']) {
            return err("Недопустимые символы в дополнительных аргументах ssh.");
        }
        let Some(flag) = t.strip_prefix('-') else {
            return err(format!(
                "Недопустимый аргумент «{t}»: разрешены только опции ssh (команды и адреса задавать нельзя)."
            ));
        };
        let mut chars = flag.chars();
        let Some(c) = chars.next() else {
            return err("Одиночный «-» недопустим.");
        };
        let rest: String = chars.collect();
        if c == 'o' {
            let spec = if rest.is_empty() {
                i += 1;
                match toks.get(i) {
                    Some(v) => v.clone(),
                    None => return err("После -o нужно указать опцию."),
                }
            } else {
                rest
            };
            check_option(&spec)?;
            out.push("-o".to_string());
            out.push(spec);
        } else if ARG_FLAGS.contains(&c) {
            let val = if rest.is_empty() {
                i += 1;
                match toks.get(i) {
                    Some(v) => v.clone(),
                    None => return err(format!("После -{c} нужно указать значение.")),
                }
            } else {
                rest
            };
            out.push(format!("-{c}"));
            out.push(val);
        } else if FLAGS.contains(&c) && flag.chars().all(|x| x == c) {
            // -v, -vv, -vvv, -4 ... (repeated letters allowed, mixed bundles are not)
            out.push(t.clone());
        } else {
            return err(format!(
                "Опция -{c} не поддерживается в дополнительных аргументах (для терминала разрешены -o, -L, -R, -D, -J, -i, -c, -m и -4 -6 -A -a -C -g -K -k -q -v -X -x -Y)."
            ));
        }
        i += 1;
    }
    Ok(out)
}

pub fn build_args(t: &SshTarget, known_hosts: &Path) -> Result<Vec<String>, ArgsError> {
    if t.host.is_empty() || t.host.starts_with('-') || t.host.contains(char::is_whitespace) {
        return err("Некорректный адрес сервера.");
    }
    if t.username.is_empty()
        || t.username.starts_with('-')
        || t.username.contains(char::is_whitespace)
    {
        return err("Некорректное имя пользователя.");
    }
    let extra = validate_extra(t.extra)?;
    let o = |s: String| ["-o".to_string(), s];
    let mut a: Vec<String> = vec!["-F".into(), "none".into()];
    for s in [
        format!("UserKnownHostsFile={}", known_hosts_value(known_hosts)?),
        "GlobalKnownHostsFile=none".into(),
        "StrictHostKeyChecking=yes".into(),
        "UpdateHostKeys=no".into(),
        "NumberOfPasswordPrompts=3".into(),
        "ServerAliveInterval=30".into(),
        "ServerAliveCountMax=4".into(),
    ] {
        a.extend(o(s));
    }
    match t.identity {
        // only the chosen key (never agent / default keys), then whatever else the server offers
        Some(k) => {
            for s in [
                "PubkeyAuthentication=yes".to_string(),
                "PreferredAuthentications=publickey,keyboard-interactive,password".into(),
                "IdentitiesOnly=yes".into(),
                "IdentityAgent=none".into(),
                "AddKeysToAgent=no".into(),
            ] {
                a.extend(o(s));
            }
            a.extend(["-i".to_string(), k.to_string_lossy().into_owned()]);
        }
        None => {
            for s in [
                "PreferredAuthentications=keyboard-interactive,password",
                "PubkeyAuthentication=no",
            ] {
                a.extend(o(s.to_string()));
            }
        }
    }
    a.extend(extra);
    a.extend([
        "-p".to_string(),
        t.port.to_string(),
        "-l".to_string(),
        t.username.to_string(),
        "-t".to_string(),
        "--".to_string(),
        t.host.to_string(),
    ]);
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn target<'a>(extra: &'a str) -> SshTarget<'a> {
        SshTarget {
            host: "example.org",
            port: 2222,
            username: "deploy",
            extra,
            identity: None,
        }
    }

    fn args(extra: &str) -> Result<Vec<String>, ArgsError> {
        build_args(&target(extra), &PathBuf::from("/data/known_hosts"))
    }

    #[test]
    fn base_arguments_are_separate_elements_in_order() {
        let a = args("").unwrap();
        assert_eq!(&a[..2], ["-F", "none"]);
        let joined = a.join(" ");
        for must in [
            "-o UserKnownHostsFile=/data/known_hosts",
            "-o GlobalKnownHostsFile=none",
            "-o StrictHostKeyChecking=yes",
            "-o UpdateHostKeys=no",
            "-o PreferredAuthentications=keyboard-interactive,password",
            "-o PubkeyAuthentication=no",
            "-o NumberOfPasswordPrompts=3",
            "-o ServerAliveInterval=30",
            "-o ServerAliveCountMax=4",
        ] {
            assert!(joined.contains(must), "missing {must}");
        }
        assert_eq!(
            &a[a.len() - 7..],
            ["-p", "2222", "-l", "deploy", "-t", "--", "example.org"]
        );
    }

    #[test]
    fn known_hosts_path_with_spaces_and_windows_separators() {
        let p = PathBuf::from(r"C:\Test Folder\Shell port\data\known_hosts");
        let a = build_args(&target(""), &p).unwrap();
        let v = a
            .iter()
            .find(|s| s.starts_with("UserKnownHostsFile="))
            .unwrap();
        assert_eq!(
            v,
            "UserKnownHostsFile=\"C:/Test Folder/Shell port/data/known_hosts\""
        );
        // each element stays one element
        assert!(a.iter().all(|s| !s.is_empty()));
        // a path with a quote is refused instead of being mangled
        assert!(build_args(&target(""), &PathBuf::from("/a\"b/kh")).is_err());
    }

    #[test]
    fn key_login_uses_only_the_chosen_key_and_keeps_the_path_as_one_argument() {
        let key = PathBuf::from("/home/me/my keys/id ed25519");
        let mut t = target("");
        t.identity = Some(&key);
        let a = build_args(&t, &PathBuf::from("/data/known_hosts")).unwrap();
        let joined = a.join(" ");
        for must in [
            "-o PubkeyAuthentication=yes",
            "-o PreferredAuthentications=publickey,keyboard-interactive,password",
            "-o IdentitiesOnly=yes",
            "-o IdentityAgent=none",
        ] {
            assert!(joined.contains(must), "{must}: {joined}");
        }
        assert!(!joined.contains("PubkeyAuthentication=no"), "{joined}");
        let i = a.iter().position(|x| x == "-i").expect("-i");
        assert_eq!(a[i + 1], "/home/me/my keys/id ed25519"); // one element: spaces need no quoting
                                                             // without a key nothing about keys is offered
        let plain = args("").unwrap().join(" ");
        assert!(plain.contains("PubkeyAuthentication=no") && !plain.contains(" -i "));
    }

    #[test]
    fn builder_has_no_parameter_for_a_password() {
        // build_args() receives only host, port, user, extras and the known_hosts path: the password
        // reaches ssh exclusively through askpass or the PTY, never through argv.
        let a = args("").unwrap();
        assert!(a.iter().all(|s| !s.contains("testpass")));
        assert_eq!(a.iter().filter(|s| s.as_str() == "-o").count(), 9);
    }

    #[test]
    fn extras_come_after_security_options_and_before_destination() {
        let a = args("-o Compression=yes -L 8080:localhost:80 -vv").unwrap();
        let pos = |s: &str| a.iter().position(|x| x == s).unwrap();
        assert!(pos("StrictHostKeyChecking=yes") < pos("Compression=yes"));
        assert!(pos("-L") < pos("-p"));
        assert_eq!(a[pos("-L") + 1], "8080:localhost:80");
        assert!(a.contains(&"-vv".to_string()));
        // the destination is always last
        assert_eq!(a.last().unwrap(), "example.org");
    }

    #[test]
    fn option_forms_are_normalised() {
        let a = args("-oServerAliveInterval=5 -o 'TCPKeepAlive yes'").unwrap();
        assert!(a.windows(2).any(|w| w == ["-o", "ServerAliveInterval=5"]));
        assert!(a.windows(2).any(|w| w == ["-o", "TCPKeepAlive yes"]));
    }

    #[test]
    fn dangerous_extras_are_rejected() {
        for bad in [
            "-o StrictHostKeyChecking=no",
            "-oStrictHostKeyChecking=no",
            "-o stricthostkeychecking no",
            "-o 'UserKnownHostsFile /dev/null'",
            "-oUserKnownHostsFile=/dev/null",
            "-o GlobalKnownHostsFile=/x",
            "-o UpdateHostKeys=yes",
            "-o HostKeyAlias=other",
            "-o PermitLocalCommand=yes",
            "-o LocalCommand=calc",
            "-o RemoteCommand=ls",
            "-o RequestTTY=no",
            "-o BatchMode=yes",
            "-o PubkeyAuthentication=yes",
            "-o Include=/etc/ssh/x",
            "-F /tmp/config",
            "-N",
            "-T",
            "-W host:22",
            "-E /tmp/log",
            "-S /tmp/ctl",
            "-s sftp",
            "-t",
            "rm -rf /",
            "ls",
            "otherhost",
            "-",
            "-o",
            "-L",
            "-vA", // mixed bundles are not accepted
            "-o 'unterminated",
            "--",
        ] {
            assert!(args(bad).is_err(), "should be rejected: {bad:?}");
        }
    }

    #[test]
    fn explicit_proxy_options_are_allowed_when_the_user_asks_for_them() {
        let a = args("-J jump.example.org -o ProxyCommand='ssh -W %h:%p gw'").unwrap();
        assert!(a.contains(&"-J".to_string()));
        assert!(a.iter().any(|s| s.starts_with("ProxyCommand=")));
    }

    #[test]
    fn host_and_user_injection_is_rejected_here_too() {
        let mut t = target("");
        t.host = "-oProxyCommand=x";
        assert!(build_args(&t, Path::new("/k")).is_err());
        let mut t = target("");
        t.username = "-l";
        assert!(build_args(&t, Path::new("/k")).is_err());
        let mut t = target("");
        t.host = "a b";
        assert!(build_args(&t, Path::new("/k")).is_err());
    }

    #[test]
    fn split_handles_quotes() {
        assert_eq!(
            split_extra("a  'b c' \"d e\" f").unwrap(),
            ["a", "b c", "d e", "f"]
        );
        assert_eq!(split_extra("").unwrap(), Vec::<String>::new());
        assert_eq!(split_extra("''").unwrap(), [""]);
    }
}
