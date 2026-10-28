//! Which local TCP port does the connection of a given `ssh` process use? The server shows that port to
//! its login shell in `SSH_CONNECTION` / `SSH_CLIENT`, which is how the terminal's shell is recognised on
//! the server without running anything there.

/// Local port of the established TCP connection of process `pid` to the remote port `remote_port`.
pub fn local_port(pid: u32, remote_port: u16) -> Option<u16> {
    #[cfg(windows)]
    {
        win::local_port(pid, remote_port)
    }
    #[cfg(target_os = "linux")]
    {
        linux::local_port(pid, remote_port)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (pid, remote_port);
        None
    }
}

/// Parse `/proc/net/tcp` (or `tcp6`): the local port of an ESTABLISHED socket whose inode is in `inodes`
/// and whose remote port is `remote_port`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_proc_net_tcp(text: &str, inodes: &[u64], remote_port: u16) -> Option<u16> {
    for l in text.lines().skip(1) {
        let t: Vec<&str> = l.split_whitespace().collect();
        if t.len() < 10 || t[3] != "01" {
            continue;
        }
        let port = |s: &str| {
            s.rsplit(':')
                .next()
                .and_then(|p| u16::from_str_radix(p, 16).ok())
        };
        let (Some(local), Some(remote)) = (port(t[1]), port(t[2])) else {
            continue;
        };
        let inode: u64 = t[9].parse().unwrap_or(0);
        if remote == remote_port && inodes.contains(&inode) {
            return Some(local);
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod linux {
    pub fn local_port(pid: u32, remote_port: u16) -> Option<u16> {
        let mut inodes = Vec::new();
        for e in std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten() {
            if let Ok(t) = std::fs::read_link(e.path()) {
                let t = t.to_string_lossy();
                if let Some(i) = t.strip_prefix("socket:[").and_then(|s| s.strip_suffix(']')) {
                    if let Ok(n) = i.parse() {
                        inodes.push(n);
                    }
                }
            }
        }
        for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(text) = std::fs::read_to_string(f) {
                if let Some(p) = super::parse_proc_net_tcp(&text, &inodes, remote_port) {
                    return Some(p);
                }
            }
        }
        None
    }
}

#[cfg(windows)]
mod win {
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetExtendedTcpTable(
            table: *mut u8,
            size: *mut u32,
            order: i32,
            af: u32,
            class: i32,
            reserved: u32,
        ) -> u32;
    }
    const TCP_TABLE_OWNER_PID_ALL: i32 = 5;
    const ESTABLISHED: u32 = 5;

    fn table(af: u32) -> Option<Vec<u8>> {
        let mut size = 0u32;
        // first call reports the needed size; the table may grow between calls, so retry a few times
        for _ in 0..5 {
            let mut buf = vec![0u8; size as usize];
            let r = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr(),
                    &mut size,
                    0,
                    af,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
            match r {
                0 => return Some(buf),
                122 => continue, // ERROR_INSUFFICIENT_BUFFER: size was updated
                _ => return None,
            }
        }
        None
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    /// The port sits in the low 16 bits, in network byte order.
    fn port(v: u32) -> u16 {
        u16::from_be((v & 0xffff) as u16)
    }

    pub fn local_port(pid: u32, remote_port: u16) -> Option<u16> {
        // IPv4 rows: state, local addr, local port, remote addr, remote port, pid (6 x u32)
        if let Some(t) = table(2) {
            let n = u32_at(&t, 0) as usize;
            for i in 0..n {
                let o = 4 + i * 24;
                if o + 24 > t.len() {
                    break;
                }
                if u32_at(&t, o) == ESTABLISHED
                    && u32_at(&t, o + 20) == pid
                    && port(u32_at(&t, o + 16)) == remote_port
                {
                    return Some(port(u32_at(&t, o + 8)));
                }
            }
        }
        // IPv6 rows: local addr[16], scope, port, remote addr[16], scope, port, state, pid (56 bytes)
        if let Some(t) = table(23) {
            let n = u32_at(&t, 0) as usize;
            for i in 0..n {
                let o = 4 + i * 56;
                if o + 56 > t.len() {
                    break;
                }
                if u32_at(&t, o + 48) == ESTABLISHED
                    && u32_at(&t, o + 52) == pid
                    && port(u32_at(&t, o + 40)) == remote_port
                {
                    return Some(port(u32_at(&t, o + 20)));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 11111 1 0 100 0 0 10 0\n   1: AC130007:C350 AC130002:0016 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0 20 4 28 10 -1\n   2: AC130007:C351 AC130003:0016 01 00000000:00000000 00:00000000 00000000  1000        0 33333 1 0 20 4 28 10 -1\n";

    #[test]
    fn finds_the_established_socket_of_the_process() {
        assert_eq!(parse_proc_net_tcp(TCP, &[22222], 22), Some(0xC350));
        assert_eq!(parse_proc_net_tcp(TCP, &[33333, 1], 22), Some(0xC351));
    }

    #[test]
    fn ignores_other_sockets_ports_and_states() {
        assert_eq!(parse_proc_net_tcp(TCP, &[11111], 22), None); // listening socket
        assert_eq!(parse_proc_net_tcp(TCP, &[22222], 2222), None); // other remote port
        assert_eq!(parse_proc_net_tcp(TCP, &[99999], 22), None); // not ours
        assert_eq!(parse_proc_net_tcp("", &[1], 22), None);
    }
}
