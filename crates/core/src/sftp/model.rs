use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// One directory entry as shown in the explorer.
#[derive(Debug, Clone, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    /// For symlinks: does the target resolve to a directory (`None` = broken or unknown)?
    pub target_is_dir: Option<bool>,
    pub size: u64,
    pub mtime: Option<u32>,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    /// The name contained bytes that are not valid UTF-8 and was shown with replacement characters.
    /// Such entries must never be used for operations: the decoded name may denote a different (or no) file.
    pub name_lossy: bool,
}

/// `rwxr-xr-x` style string from the permission bits of a mode.
pub fn mode_string(mode: u32) -> String {
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    let mut s: String = bits
        .iter()
        .map(|(b, c)| if mode & b != 0 { *c } else { '-' })
        .collect();
    // setuid / setgid / sticky
    let patch = |s: &mut String, idx: usize, on: bool, lower: char, upper: char| {
        if on {
            let x = s.as_bytes()[idx] == b'x';
            s.replace_range(idx..idx + 1, &(if x { lower } else { upper }).to_string());
        }
    };
    patch(&mut s, 2, mode & 0o4000 != 0, 's', 'S');
    patch(&mut s, 5, mode & 0o2000 != 0, 's', 'S');
    patch(&mut s, 8, mode & 0o1000 != 0, 't', 'T');
    s
}

#[cfg(test)]
mod tests {
    use super::mode_string;

    #[test]
    fn modes() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_string(0o644), "rw-r--r--");
        assert_eq!(mode_string(0o000), "---------");
        assert_eq!(mode_string(0o4755), "rwsr-xr-x");
        assert_eq!(mode_string(0o1777), "rwxrwxrwt");
        assert_eq!(mode_string(0o2644), "rw-r-Sr--");
        // file-type bits in the high part must not matter
        assert_eq!(mode_string(0o100644), "rw-r--r--");
    }
}
