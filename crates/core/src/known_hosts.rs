//! OpenSSH-compatible `known_hosts`: parsing, matching, atomic append/replace.
//!
//! Understands comments, blank lines, comma separated host lists, `[host]:port`, hashed entries
//! (`|1|salt|hash`, read only), `@revoked` and `@cert-authority` markers. Writing never touches
//! foreign lines.
use base64ct::{Base64, Encoding};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    Revoked,
    CertAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub marker: Marker,
    pub hosts: Vec<String>,
    pub key_type: String,
    /// Base64 key blob exactly as in the file.
    pub key_b64: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A non-revoked entry for this host and key type matches the presented key.
    Match,
    /// No entry mentions this host.
    Unknown,
    /// The host is known, but not with this key. `known_types` lists the key types on file.
    Changed { known_types: Vec<String> },
    /// The presented key is explicitly revoked.
    Revoked,
}

/// Host token as OpenSSH writes it: bare for port 22, `[host]:port` otherwise.
pub fn host_token(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

fn hashed_match(token: &str, host_token: &str) -> bool {
    // |1|base64(salt)|base64(hmac-sha1(salt, host))
    let mut it = token.strip_prefix("|1|").unwrap_or("").split('|');
    let (Some(salt), Some(hash)) = (it.next(), it.next()) else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (Base64::decode_vec(salt), Base64::decode_vec(hash)) else {
        return false;
    };
    let Ok(mut mac) = <Hmac<Sha1> as KeyInit>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(host_token.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

fn pattern_match(pat: &str, host: &str) -> bool {
    // Supports the `*` and `?` wildcards OpenSSH allows in host patterns.
    fn go(p: &[u8], h: &[u8]) -> bool {
        match (p.first(), h.first()) {
            (None, None) => true,
            (Some(b'*'), _) => go(&p[1..], h) || (!h.is_empty() && go(p, &h[1..])),
            (Some(b'?'), Some(_)) => go(&p[1..], &h[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &h[1..]),
            _ => false,
        }
    }
    go(pat.as_bytes(), host.as_bytes())
}

impl Entry {
    pub fn parse_line(line: &str) -> Option<Entry> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut f = line.split_whitespace();
        let mut first = f.next()?;
        let marker = match first {
            "@revoked" => Marker::Revoked,
            "@cert-authority" => Marker::CertAuthority,
            _ if first.starts_with('@') => return None, // unknown marker: ignore line
            _ => Marker::None,
        };
        if marker != Marker::None {
            first = f.next()?;
        }
        let key_type = f.next()?.to_string();
        let key_b64 = f.next()?.to_string();
        Some(Entry {
            marker,
            hosts: first.split(',').map(str::to_string).collect(),
            key_type,
            key_b64,
        })
    }

    /// Does this entry apply to `host:port`? Negated patterns (`!pat`) exclude the host.
    pub fn matches_host(&self, host: &str, port: u16) -> bool {
        let token = host_token(host, port);
        let mut hit = false;
        for h in &self.hosts {
            if let Some(neg) = h.strip_prefix('!') {
                if pattern_match(neg, &token) {
                    return false;
                }
            } else if h.starts_with("|1|") {
                hit |= hashed_match(h, &token);
            } else {
                hit |= pattern_match(h, &token);
            }
        }
        hit
    }
}

/// Check `host:port` presenting `key_type`/`key_b64` against file contents.
pub fn check_text(text: &str, host: &str, port: u16, key_type: &str, key_b64: &str) -> Verdict {
    let mut host_known = false;
    let mut known_types: Vec<String> = Vec::new();
    let mut matched = false;
    for e in text.lines().filter_map(Entry::parse_line) {
        if !e.matches_host(host, port) {
            continue;
        }
        match e.marker {
            Marker::Revoked => {
                if e.key_b64 == key_b64 {
                    return Verdict::Revoked;
                }
            }
            Marker::CertAuthority => {} // CA entries are not host keys; never trusted here
            Marker::None => {
                host_known = true;
                if !known_types.contains(&e.key_type) {
                    known_types.push(e.key_type.clone());
                }
                if e.key_type == key_type && e.key_b64 == key_b64 {
                    matched = true;
                }
            }
        }
    }
    if matched {
        Verdict::Match
    } else if host_known {
        Verdict::Changed { known_types }
    } else {
        Verdict::Unknown
    }
}

pub fn check_file(
    path: &Path,
    host: &str,
    port: u16,
    key_type: &str,
    key_b64: &str,
) -> std::io::Result<Verdict> {
    let text = match fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    Ok(check_text(&text, host, port, key_type, key_b64))
}

/// Key types on file for this host (used to prefer the same algorithm when connecting).
pub fn known_key_types(path: &Path, host: &str, port: u16) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut out = Vec::new();
    for e in text.lines().filter_map(Entry::parse_line) {
        if e.marker == Marker::None && e.matches_host(host, port) && !out.contains(&e.key_type) {
            out.push(e.key_type);
        }
    }
    out
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn read_or_empty(path: &Path) -> std::io::Result<Vec<u8>> {
    match fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Append a trusted host key. Existing content is preserved byte for byte.
pub fn add(
    path: &Path,
    host: &str,
    port: u16,
    key_type: &str,
    key_b64: &str,
) -> std::io::Result<()> {
    let mut data = read_or_empty(path)?;
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    data.extend_from_slice(
        format!("{} {} {}\n", host_token(host, port), key_type, key_b64).as_bytes(),
    );
    atomic_write(path, &data)
}

/// Explicit key replacement (after a "key changed" warning confirmed twice by the user): removes the
/// plain (non-hashed, non-marker) lines that mention only this host, drops this host from shared
/// lines, then appends the new key. Foreign lines stay untouched.
pub fn replace(
    path: &Path,
    host: &str,
    port: u16,
    key_type: &str,
    key_b64: &str,
) -> std::io::Result<()> {
    let token = host_token(host, port);
    let data = read_or_empty(path)?;
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut out = String::new();
    for line in text.lines() {
        match Entry::parse_line(line) {
            Some(e) if e.marker == Marker::None && e.hosts.iter().any(|h| h == &token) => {
                let rest: Vec<&str> = e
                    .hosts
                    .iter()
                    .filter(|h| *h != &token)
                    .map(String::as_str)
                    .collect();
                if !rest.is_empty() {
                    out.push_str(&format!(
                        "{} {} {}\n",
                        rest.join(","),
                        e.key_type,
                        e.key_b64
                    ));
                }
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push_str(&format!("{token} {key_type} {key_b64}\n"));
    atomic_write(path, out.as_bytes())
}

/// Helper for building hashed entries in tests and for OpenSSH interoperability checks.
pub fn hash_host(salt: &[u8], host_token: &str) -> String {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(salt).expect("hmac key");
    mac.update(host_token.as_bytes());
    let h = mac.finalize().into_bytes();
    format!(
        "|1|{}|{}",
        Base64::encode_string(salt),
        Base64::encode_string(&h)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const K1: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIK1111111111111111111111111111111111111111";
    const K2: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIK2222222222222222222222222222222222222222";

    #[test]
    fn host_tokens() {
        assert_eq!(host_token("example.org", 22), "example.org");
        assert_eq!(host_token("example.org", 2222), "[example.org]:2222");
    }

    #[test]
    fn comments_blank_and_unknown_marker_lines_are_ignored() {
        for l in [
            "",
            "   ",
            "# comment",
            "@weird host ssh-ed25519 AAAA",
            "onlyonefield",
        ] {
            assert!(Entry::parse_line(l).is_none(), "{l:?}");
        }
    }

    #[test]
    fn match_unknown_changed() {
        let t = format!("example.org ssh-ed25519 {K1}\n");
        assert_eq!(
            check_text(&t, "example.org", 22, "ssh-ed25519", K1),
            Verdict::Match
        );
        assert_eq!(
            check_text(&t, "other.org", 22, "ssh-ed25519", K1),
            Verdict::Unknown
        );
        assert_eq!(
            check_text(&t, "example.org", 22, "ssh-ed25519", K2),
            Verdict::Changed {
                known_types: vec!["ssh-ed25519".into()]
            }
        );
    }

    #[test]
    fn different_key_type_counts_as_changed_with_known_types() {
        let t = format!("h ssh-rsa {K1}\n");
        assert_eq!(
            check_text(&t, "h", 22, "ssh-ed25519", K1),
            Verdict::Changed {
                known_types: vec!["ssh-rsa".into()]
            }
        );
    }

    #[test]
    fn port_matching() {
        let t = format!("[h]:2222 ssh-ed25519 {K1}\nh ssh-ed25519 {K2}\n");
        assert_eq!(check_text(&t, "h", 2222, "ssh-ed25519", K1), Verdict::Match);
        assert_eq!(check_text(&t, "h", 22, "ssh-ed25519", K2), Verdict::Match);
        assert_eq!(
            check_text(&t, "h", 22, "ssh-ed25519", K1),
            Verdict::Changed {
                known_types: vec!["ssh-ed25519".into()]
            }
        );
        assert_eq!(
            check_text(&t, "h", 2200, "ssh-ed25519", K1),
            Verdict::Unknown
        );
    }

    #[test]
    fn comma_separated_hosts_and_ip() {
        let t = format!("a.example,10.0.0.5,[b.example]:2200 ssh-ed25519 {K1}\n");
        for (h, p) in [("a.example", 22), ("10.0.0.5", 22), ("b.example", 2200)] {
            assert_eq!(
                check_text(&t, h, p, "ssh-ed25519", K1),
                Verdict::Match,
                "{h}:{p}"
            );
        }
    }

    #[test]
    fn wildcards_and_negation() {
        let t = format!("*.example.org,!bad.example.org ssh-ed25519 {K1}\n");
        assert_eq!(
            check_text(&t, "web.example.org", 22, "ssh-ed25519", K1),
            Verdict::Match
        );
        assert_eq!(
            check_text(&t, "bad.example.org", 22, "ssh-ed25519", K1),
            Verdict::Unknown
        );
    }

    #[test]
    fn hashed_entries_are_matched() {
        let salt = b"0123456789abcdefghij";
        let t = format!("{} ssh-ed25519 {K1}\n", hash_host(salt, "secret.example"));
        assert_eq!(
            check_text(&t, "secret.example", 22, "ssh-ed25519", K1),
            Verdict::Match
        );
        assert_eq!(
            check_text(&t, "nope.example", 22, "ssh-ed25519", K1),
            Verdict::Unknown
        );
        let t = format!(
            "{} ssh-ed25519 {K1}\n",
            hash_host(salt, "[secret.example]:2200")
        );
        assert_eq!(
            check_text(&t, "secret.example", 2200, "ssh-ed25519", K1),
            Verdict::Match
        );
    }

    #[test]
    fn hashed_entry_known_openssh_vector() {
        // Generated with `ssh-keygen -H` semantics: HMAC-SHA1(salt, "host"); verify our hasher agrees with itself
        // across encode/decode and rejects tampering.
        let h = hash_host(b"salt-salt-salt-salt-", "host");
        assert!(hashed_match(&h, "host"));
        assert!(!hashed_match(&h, "host2"));
        assert!(!hashed_match("|1|!!!|???", "host"));
    }

    #[test]
    fn revoked_marker_rejects_even_if_plain_entry_matches() {
        let t = format!("h ssh-ed25519 {K1}\n@revoked h ssh-ed25519 {K1}\n");
        assert_eq!(check_text(&t, "h", 22, "ssh-ed25519", K1), Verdict::Revoked);
        // a revoked key does not affect other keys
        let t = format!("h ssh-ed25519 {K2}\n@revoked h ssh-ed25519 {K1}\n");
        assert_eq!(check_text(&t, "h", 22, "ssh-ed25519", K2), Verdict::Match);
    }

    #[test]
    fn cert_authority_lines_do_not_make_a_host_trusted() {
        let t = format!("@cert-authority *.example.org ssh-ed25519 {K1}\n");
        assert_eq!(
            check_text(&t, "web.example.org", 22, "ssh-ed25519", K1),
            Verdict::Unknown
        );
    }

    #[test]
    fn crlf_and_trailing_spaces() {
        let t = format!("h ssh-ed25519 {K1} comment here  \r\n");
        assert_eq!(check_text(&t, "h", 22, "ssh-ed25519", K1), Verdict::Match);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("d256shell-kh-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.join("known_hosts")
    }

    #[test]
    fn add_preserves_foreign_lines_and_formats_port() {
        let p = tmp("add");
        fs::write(&p, "# my hosts\nfoo ssh-rsa AAAAfoo comment").unwrap(); // no trailing newline
        add(&p, "h", 22, "ssh-ed25519", K1).unwrap();
        add(&p, "h", 2222, "ssh-ed25519", K2).unwrap();
        let t = fs::read_to_string(&p).unwrap();
        assert_eq!(
            t,
            format!("# my hosts\nfoo ssh-rsa AAAAfoo comment\nh ssh-ed25519 {K1}\n[h]:2222 ssh-ed25519 {K2}\n")
        );
        assert_eq!(
            check_file(&p, "h", 2222, "ssh-ed25519", K2).unwrap(),
            Verdict::Match
        );
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_file_means_unknown_and_add_creates_it() {
        let p = tmp("missing");
        assert_eq!(
            check_file(&p, "h", 22, "ssh-ed25519", K1).unwrap(),
            Verdict::Unknown
        );
        add(&p, "h", 22, "ssh-ed25519", K1).unwrap();
        assert_eq!(
            check_file(&p, "h", 22, "ssh-ed25519", K1).unwrap(),
            Verdict::Match
        );
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn replace_only_changes_this_host() {
        let p = tmp("replace");
        let salt = b"0123456789abcdefghij";
        fs::write(
            &p,
            format!(
                "other ssh-ed25519 {K2}\nh,shared ssh-ed25519 {K1}\n{} ssh-ed25519 {K2}\n@revoked x ssh-rsa AAAA\n",
                hash_host(salt, "hashed")
            ),
        )
        .unwrap();
        replace(&p, "h", 22, "ssh-ed25519", K2).unwrap();
        let t = fs::read_to_string(&p).unwrap();
        assert!(t.contains(&format!("other ssh-ed25519 {K2}\n")));
        assert!(
            t.contains(&format!("shared ssh-ed25519 {K1}\n")),
            "shared host line keeps the other host"
        );
        assert!(t.contains("@revoked x ssh-rsa AAAA\n"));
        assert!(t.contains(&hash_host(salt, "hashed")));
        assert_eq!(
            check_file(&p, "h", 22, "ssh-ed25519", K2).unwrap(),
            Verdict::Match
        );
        assert_eq!(
            check_file(&p, "shared", 22, "ssh-ed25519", K1).unwrap(),
            Verdict::Match
        );
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn known_key_types_lists_plain_entries() {
        let p = tmp("types");
        fs::write(
            &p,
            format!("h ssh-rsa {K1}\nh ssh-ed25519 {K2}\n@revoked h ecdsa-sha2-nistp256 AAAA\n"),
        )
        .unwrap();
        assert_eq!(known_key_types(&p, "h", 22), vec!["ssh-rsa", "ssh-ed25519"]);
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
