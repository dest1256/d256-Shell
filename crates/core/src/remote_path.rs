//! Remote (POSIX) path helpers. Remote paths are plain `&str`; `std::path::Path` is never used for them
//! (on Windows it would treat `\` and drive letters specially).

/// Join `base` and `name`. `name` must be a single path component (see [`is_valid_name`]).
pub fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// Parent directory of an absolute normalized path. The parent of `/` is `/`.
pub fn parent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    if p.is_empty() && path.starts_with('/') {
        return "/".to_string();
    }
    match p.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
    }
}

/// Last component (`""` for `/`).
pub fn file_name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/').next().unwrap_or("")
}

/// Collapse `//`, `.` and `..` lexically. Relative paths stay relative; `..` above the root of an absolute
/// path is dropped. (Symlinks are not resolved: use the server's `realpath` for that.)
pub fn normalize(path: &str) -> String {
    let abs = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(l) if *l != "..") {
                    out.pop();
                } else if !abs {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    let body = out.join("/");
    match (abs, body.is_empty()) {
        (true, _) => format!("/{body}"),
        (false, true) => ".".to_string(),
        (false, false) => body,
    }
}

/// Is `name` acceptable as a single new file/folder name?
/// Rejects empty, `.`/`..`, anything containing `/` or NUL, and control characters.
/// (Names starting with `-` are fine for SFTP: no shell is involved.)
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\0'])
        && !name.chars().any(char::is_control)
        && name.len() <= 255
}

/// Is `path` equal to `ancestor` or located inside it (component-wise, so `/a/bc` is not inside `/a/b`)?
pub fn is_same_or_inside(path: &str, ancestor: &str) -> bool {
    let (p, a) = (normalize(path), normalize(ancestor));
    if a == "/" {
        return p.starts_with('/');
    }
    p == a || p.starts_with(&format!("{a}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_cases() {
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(join("/a/", "b"), "/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(join("", "b"), "b");
        assert_eq!(join("/a", "-x y"), "/a/-x y");
    }

    #[test]
    fn parent_cases() {
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a/b/"), "/a");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("a"), ".");
    }

    #[test]
    fn file_name_cases() {
        assert_eq!(file_name("/a/b.txt"), "b.txt");
        assert_eq!(file_name("/a/b/"), "b");
        assert_eq!(file_name("/"), "");
    }

    #[test]
    fn normalize_cases() {
        assert_eq!(normalize("/a//b/./c/../d"), "/a/b/d");
        assert_eq!(normalize("/.."), "/");
        assert_eq!(normalize("/a/../.."), "/");
        assert_eq!(normalize("a/../.."), "..");
        assert_eq!(normalize(""), ".");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize("///"), "/");
        assert_eq!(normalize("/a/b/"), "/a/b");
    }

    #[test]
    fn names() {
        for ok in ["a", "файл", "with space", "-dash", ".hidden", "it's $x *"] {
            assert!(is_valid_name(ok), "{ok:?}");
        }
        for bad in ["", ".", "..", "a/b", "a\0b", "a\nb", &"x".repeat(256)] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn inside() {
        assert!(is_same_or_inside("/a/b", "/a"));
        assert!(is_same_or_inside("/a", "/a"));
        assert!(!is_same_or_inside("/a/bc", "/a/b"));
        assert!(is_same_or_inside("/a/b/../c", "/a"));
        assert!(!is_same_or_inside("/a/b/../../c", "/a"));
        assert!(is_same_or_inside("/anything", "/"));
    }
}
