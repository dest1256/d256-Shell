//! Server load monitor. Nothing is executed on the server: the numbers come from reading the ordinary files
//! `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`, `/proc/uptime` and `/proc/net/dev` over SFTP.
//! The UI turns two consecutive samples into percentages and rates.
use serde_json::{json, Value};

/// The `/proc` files of one sample (a missing file is `None`).
#[derive(Default)]
pub struct RawSample<'a> {
    pub stat: Option<&'a str>,
    pub meminfo: Option<&'a str>,
    pub loadavg: Option<&'a str>,
    pub uptime: Option<&'a str>,
    pub netdev: Option<&'a str>,
    pub diskstats: Option<&'a str>,
    pub cpuinfo: Option<&'a str>,
}

fn nums(line: &str) -> Vec<u64> {
    line.split_whitespace()
        .skip(1)
        .filter_map(|t| t.parse().ok())
        .collect()
}

/// `[busy, total]` jiffies from a `cpu` line (idle and iowait count as idle).
fn cpu_pair(line: &str) -> Option<[u64; 2]> {
    let v = nums(line);
    if v.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest time is already inside user)
    let total: u64 = v.iter().take(8).sum();
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some([total.saturating_sub(idle), total])
}

fn meminfo_kb(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find(|l| l.starts_with(key) && l[key.len()..].starts_with(':'))
        .and_then(|l| l[key.len() + 1..].split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

/// Sum of received / sent bytes over all interfaces except loopback.
fn net_totals(text: &str) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for l in text.lines() {
        let Some((name, rest)) = l.split_once(':') else {
            continue;
        };
        if name.trim() == "lo" {
            continue;
        }
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if f.len() >= 9 {
            rx = rx.saturating_add(f[0]);
            tx = tx.saturating_add(f[8]);
        }
    }
    (rx, tx)
}

/// Parse one sample. CPU values are cumulative counters; the UI computes deltas.
pub fn parse_sample(raw: &RawSample<'_>) -> Value {
    let mut cpu = Value::Null;
    let mut cores = Vec::new();
    if let Some(stat) = raw.stat {
        for l in stat.lines() {
            if l.starts_with("cpu ") {
                cpu = cpu_pair(l).map_or(Value::Null, |p| json!(p));
            } else if l.starts_with("cpu") {
                if let Some(p) = cpu_pair(l) {
                    cores.push(json!(p));
                }
            }
        }
    }
    let mem = raw.meminfo.map(|m| {
        let total = meminfo_kb(m, "MemTotal").unwrap_or(0);
        let free = meminfo_kb(m, "MemFree").unwrap_or(0);
        let cached =
            meminfo_kb(m, "Cached").unwrap_or(0) + meminfo_kb(m, "SReclaimable").unwrap_or(0);
        let buffers = meminfo_kb(m, "Buffers").unwrap_or(0);
        // MemAvailable exists since Linux 3.14; older kernels: an estimate
        let available = meminfo_kb(m, "MemAvailable").unwrap_or(free + cached + buffers);
        json!({
            "total": total * 1024,
            "available": available.min(total) * 1024,
            "free": free.min(total) * 1024,
            "cached": (cached + buffers) * 1024,
            "swap_total": meminfo_kb(m, "SwapTotal").unwrap_or(0) * 1024,
            "swap_free": meminfo_kb(m, "SwapFree").unwrap_or(0) * 1024,
        })
    });
    let load = raw.loadavg.map(|l| {
        let t: Vec<&str> = l.split_whitespace().collect();
        let f = |i: usize| t.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let (running, procs) = t
            .get(3)
            .and_then(|v| v.split_once('/'))
            .map(|(a, b)| (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().unwrap_or(0)))
            .unwrap_or((0, 0));
        json!({ "l1": f(0), "l5": f(1), "l15": f(2), "running": running, "procs": procs })
    });
    let uptime = raw
        .uptime
        .and_then(|u| u.split_whitespace().next())
        .and_then(|v| v.parse::<f64>().ok());
    let net = raw.netdev.map(|n| {
        let (rx, tx) = net_totals(n);
        json!({ "rx": rx, "tx": tx })
    });
    let disks = raw.diskstats.map(parse_disks);
    let ifaces = raw.netdev.map(parse_ifaces);
    let cpu_mhz = raw.cpuinfo.and_then(avg_mhz);
    json!({ "cpu": cpu, "cores": cores, "mem": mem, "load": load, "uptime": uptime, "net": net, "disks": disks, "ifaces": ifaces, "cpu_mhz": cpu_mhz })
}

/// Whole block devices worth showing: no loop/ram/optical devices and no partitions (their traffic is
/// already counted in the parent device).
fn is_physical_disk(name: &str) -> bool {
    let skip = ["loop", "ram", "fd", "sr", "zram", "nbd"];
    if skip.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    let digits_end = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // sda1, vdb2, xvda1, hda3
    for p in ["sd", "hd", "vd", "xvd"] {
        if let Some(rest) = name.strip_prefix(p) {
            let letters = rest.trim_end_matches(|c: char| c.is_ascii_digit());
            if !letters.is_empty()
                && letters.len() < rest.len()
                && digits_end(&rest[letters.len()..])
            {
                return false;
            }
        }
    }
    // nvme0n1p1, mmcblk0p1, md0p1
    for p in ["nvme", "mmcblk", "md"] {
        if name.starts_with(p) {
            if let Some(i) = name.rfind('p') {
                if i > p.len()
                    && digits_end(&name[i + 1..])
                    && name[..i].bytes().last().is_some_and(|b| b.is_ascii_digit())
                {
                    return false;
                }
            }
        }
    }
    true
}

/// `/proc/diskstats` -> cumulative counters per physical disk. A sector is always 512 bytes there.
/// `[reads, sectors_read, ms_reading, writes, sectors_written, ms_writing, ms_doing_io, in_flight]`.
fn parse_disks(text: &str) -> Value {
    let mut out = Vec::new();
    for l in text.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        if t.len() < 14 || !is_physical_disk(t[2]) {
            continue;
        }
        let n = |i: usize| t[i].parse::<u64>().unwrap_or(0);
        // idle devices (never used) only add noise
        if n(3) == 0 && n(7) == 0 {
            continue;
        }
        out.push(json!({
            "name": t[2],
            "c": [n(3), n(5), n(6), n(7), n(9), n(10), n(12), n(11)],
        }));
    }
    Value::Array(out)
}

/// `/proc/net/dev` -> cumulative bytes per interface (loopback left out).
fn parse_ifaces(text: &str) -> Value {
    let mut out = Vec::new();
    for l in text.lines() {
        let Some((name, rest)) = l.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" || name.is_empty() {
            continue;
        }
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        if f.len() >= 9 {
            out.push(json!({ "name": name, "rx": f[0], "tx": f[8] }));
        }
    }
    Value::Array(out)
}

/// Average current frequency (MHz) of all cores from `/proc/cpuinfo` (`cpu MHz` lines); None if not reported.
fn avg_mhz(text: &str) -> Option<f64> {
    let v: Vec<f64> = text
        .lines()
        .filter(|l| l.starts_with("cpu MHz"))
        .filter_map(|l| l.split_once(':')?.1.trim().parse().ok())
        .collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Static facts about one disk from `/sys/block/<name>/...` (each file may be missing).
pub fn parse_disk_info(
    size_sectors: Option<&str>,
    rotational: Option<&str>,
    model: Option<&str>,
) -> Value {
    let size = size_sectors
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|s| s * 512);
    let kind = match rotational.map(str::trim) {
        Some("1") => "HDD",
        Some("0") => "SSD",
        _ => "",
    };
    json!({ "size": size, "kind": kind, "model": model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()) })
}

/// Static facts about one network interface from `/sys/class/net/<name>/...`.
pub fn parse_iface_info(speed: Option<&str>, state: Option<&str>, mac: Option<&str>) -> Value {
    // speed is -1 (or unreadable) for virtual / down links
    let speed = speed
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|s| *s > 0);
    json!({ "speed_mbit": speed, "state": state.map(|s| s.trim().to_string()), "mac": mac.map(|m| m.trim().to_string()) })
}

/// One-off description of the machine: host name, system name, CPU model.
pub fn parse_info(
    hostname: Option<&str>,
    os_release: Option<&str>,
    cpuinfo: Option<&str>,
) -> Value {
    let os = os_release.and_then(|t| {
        t.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim().trim_matches('"').to_string())
    });
    let model = cpuinfo.and_then(|t| {
        t.lines()
            .find(|l| {
                l.starts_with("model name") || l.starts_with("Model") || l.starts_with("Hardware")
            })
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    });
    json!({ "hostname": hostname.map(|h| h.trim().to_string()), "os": os, "cpu_model": model })
}

use crate::sftp::SftpConnection;
use tokio::io::AsyncReadExt;

impl SftpConnection {
    /// Read a small text file (a `/proc` entry reports size 0, so it is read until EOF). `None` if unreadable.
    pub async fn read_small(&self, path: &str, max: u64) -> Option<String> {
        let f = self.sftp.open(path.to_string()).await.ok()?;
        let mut buf = Vec::new();
        f.take(max).read_to_end(&mut buf).await.ok()?;
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    /// One sample of the server load (`with_info`: also the machine description, read once by the UI).
    pub async fn monitor_sample(&self, with_info: bool) -> Option<Value> {
        const MAX: u64 = 256 * 1024;
        let (stat, mem, load, up, net, disks, cpuinfo) = tokio::join!(
            self.read_small("/proc/stat", MAX),
            self.read_small("/proc/meminfo", MAX),
            self.read_small("/proc/loadavg", 4096),
            self.read_small("/proc/uptime", 4096),
            self.read_small("/proc/net/dev", MAX),
            self.read_small("/proc/diskstats", MAX),
            self.read_small("/proc/cpuinfo", MAX),
        );
        if stat.is_none() && mem.is_none() {
            return None;
        }
        let mut v = parse_sample(&RawSample {
            stat: stat.as_deref(),
            meminfo: mem.as_deref(),
            loadavg: load.as_deref(),
            uptime: up.as_deref(),
            netdev: net.as_deref(),
            diskstats: disks.as_deref(),
            cpuinfo: cpuinfo.as_deref(),
        });
        if with_info {
            let (h, os) = tokio::join!(
                self.read_small("/proc/sys/kernel/hostname", 4096),
                self.read_small("/etc/os-release", 16 * 1024),
            );
            let mut info = parse_info(h.as_deref(), os.as_deref(), cpuinfo.as_deref());
            info["devices"] = self.device_info(&v).await;
            v["info"] = info;
        }
        Some(v)
    }

    /// Capacity / type / model of every disk and speed of every interface found in `sample` (read once).
    async fn device_info(&self, sample: &Value) -> Value {
        let names = |k: &str| -> Vec<String> {
            sample[k]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|d| d["name"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        // names come from /proc and are used in a path: allow only plain device names
        let safe = |n: &String| {
            !n.is_empty()
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:@".contains(c))
                && !n.starts_with('.')
        };
        let mut disks = serde_json::Map::new();
        for n in names("disks").iter().filter(|n| safe(n)) {
            let (p_size, p_rot, p_model) = (
                format!("/sys/block/{n}/size"),
                format!("/sys/block/{n}/queue/rotational"),
                format!("/sys/block/{n}/device/model"),
            );
            let (size, rot, model) = tokio::join!(
                self.read_small(&p_size, 64),
                self.read_small(&p_rot, 64),
                self.read_small(&p_model, 256)
            );
            disks.insert(
                n.clone(),
                parse_disk_info(size.as_deref(), rot.as_deref(), model.as_deref()),
            );
        }
        let mut ifaces = serde_json::Map::new();
        for n in names("ifaces").iter().filter(|n| safe(n)) {
            let (p_speed, p_state, p_mac) = (
                format!("/sys/class/net/{n}/speed"),
                format!("/sys/class/net/{n}/operstate"),
                format!("/sys/class/net/{n}/address"),
            );
            let (speed, state, mac) = tokio::join!(
                self.read_small(&p_speed, 64),
                self.read_small(&p_state, 64),
                self.read_small(&p_mac, 64)
            );
            ifaces.insert(
                n.clone(),
                parse_iface_info(speed.as_deref(), state.as_deref(), mac.as_deref()),
            );
        }
        json!({ "disks": disks, "ifaces": ifaces })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 60 0 20 400 20 0 0 0 0 0\ncpu1 40 0 30 400 30 0 0 0 0 0\nintr 1\n";
    const MEM: &str = "MemTotal:        8000 kB\nMemFree:         1000 kB\nMemAvailable:    4000 kB\nBuffers:          100 kB\nCached:           900 kB\nSReclaimable:     100 kB\nSwapTotal:       2000 kB\nSwapFree:        1500 kB\n";
    const NET: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n  eth1: 500 5 0 0 0 0 0 0 100 1 0 0 0 0 0 0\n";

    #[test]
    fn parses_a_full_sample() {
        let v = parse_sample(&RawSample {
            stat: Some(STAT),
            meminfo: Some(MEM),
            loadavg: Some("0.52 0.40 0.30 2/345 6789\n"),
            uptime: Some("12345.67 9999.00\n"),
            netdev: Some(NET),
            diskstats: Some(DISKS),
            cpuinfo: Some(
                "processor : 0\ncpu MHz\t\t: 3000.000\nprocessor : 1\ncpu MHz\t\t: 4000.000\n",
            ),
        });
        assert_eq!(v["cpu"], json!([150, 1000])); // busy = total - idle - iowait
        assert_eq!(v["cores"].as_array().unwrap().len(), 2);
        assert_eq!(v["cores"][0], json!([80, 500]));
        assert_eq!(v["mem"]["total"], 8000 * 1024);
        assert_eq!(v["mem"]["available"], 4000 * 1024);
        assert_eq!(v["mem"]["swap_total"], 2000 * 1024);
        assert_eq!(v["mem"]["swap_free"], 1500 * 1024);
        assert_eq!(v["load"]["l1"], 0.52);
        assert_eq!(v["load"]["running"], 2);
        assert_eq!(v["load"]["procs"], 345);
        assert_eq!(v["uptime"], 12345.67);
        assert_eq!(v["net"], json!({ "rx": 1500, "tx": 2100 })); // loopback excluded
    }

    const DISKS: &str = "   7       0 loop0 10 0 20 5 0 0 0 0 0 5 5\n   8       0 sda 1000 10 20000 500 2000 20 40000 900 3 1200 1400 0 0 0 0\n   8       1 sda1 900 10 18000 400 1900 20 38000 800 0 1100 1200 0 0 0 0\n 259       0 nvme0n1 50 0 400 10 60 0 480 20 0 30 30 0 0 0 0\n 259       1 nvme0n1p1 40 0 300 8 50 0 400 15 0 25 25 0 0 0 0\n 253       0 dm-0 500 0 4000 100 700 0 5600 300 1 400 400 0 0 0 0\n   8      16 sdb 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

    #[test]
    fn disks_skip_partitions_loop_and_idle_devices() {
        let v = parse_sample(&RawSample {
            diskstats: Some(DISKS),
            ..Default::default()
        });
        let names: Vec<&str> = v["disks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["sda", "nvme0n1", "dm-0"]);
        // reads, sectors read, ms reading, writes, sectors written, ms writing, ms doing io, in flight
        assert_eq!(
            v["disks"][0]["c"],
            json!([1000, 20000, 500, 2000, 40000, 900, 1200, 3])
        );
    }

    #[test]
    fn partition_name_rules() {
        for p in [
            "sda1",
            "vdb12",
            "xvda2",
            "hda3",
            "nvme0n1p2",
            "mmcblk0p1",
            "md0p1",
            "loop3",
            "ram0",
            "sr0",
        ] {
            assert!(!is_physical_disk(p), "{p}");
        }
        for d in [
            "sda", "vdb", "xvda", "nvme0n1", "nvme1n2", "mmcblk0", "md0", "dm-0", "dm-12", "sdaa",
        ] {
            assert!(is_physical_disk(d), "{d}");
        }
    }
    #[test]
    fn missing_files_and_garbage_do_not_break_it() {
        let v = parse_sample(&RawSample::default());
        assert!(
            v["cpu"].is_null() && v["mem"].is_null() && v["load"].is_null() && v["net"].is_null()
        );
        let v = parse_sample(&RawSample {
            stat: Some("garbage\ncpu x y\n"),
            meminfo: Some("nothing useful"),
            loadavg: Some(""),
            uptime: Some("abc"),
            netdev: Some("no colon here"),
            diskstats: Some("garbage"),
            cpuinfo: Some("nothing"),
        });
        assert!(v["cpu"].is_null());
        assert_eq!(v["mem"]["total"], 0);
        assert!(v["uptime"].is_null());
        assert_eq!(v["net"], json!({ "rx": 0, "tx": 0 }));
    }

    #[test]
    fn old_kernels_without_memavailable_get_an_estimate() {
        let m = "MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 150 kB\n";
        let v = parse_sample(&RawSample {
            meminfo: Some(m),
            ..Default::default()
        });
        assert_eq!(v["mem"]["available"], 300 * 1024);
    }

    #[test]
    fn per_interface_free_memory_and_cpu_frequency() {
        let v = parse_sample(&RawSample {
            meminfo: Some(MEM),
            netdev: Some(NET),
            cpuinfo: Some("cpu MHz\t\t: 3000.5\ncpu MHz\t\t: 4000.5\n"),
            ..Default::default()
        });
        assert_eq!(v["mem"]["free"], 1000 * 1024);
        let names: Vec<&str> = v["ifaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["eth0", "eth1"]); // loopback left out
        assert_eq!(v["ifaces"][0]["rx"], 1000);
        assert_eq!(v["ifaces"][0]["tx"], 2000);
        assert_eq!(v["cpu_mhz"], 3500.5);
        assert!(parse_sample(&RawSample::default())["cpu_mhz"].is_null());
    }

    #[test]
    fn device_facts() {
        let d = parse_disk_info(
            Some("7814037168\n"),
            Some("1\n"),
            Some("ST4000LM024-2U81  \n"),
        );
        assert_eq!(d["size"], 7814037168u64 * 512);
        assert_eq!(d["kind"], "HDD");
        assert_eq!(d["model"], "ST4000LM024-2U81");
        assert_eq!(parse_disk_info(None, Some("0"), None)["kind"], "SSD");
        assert_eq!(
            parse_disk_info(None, None, Some(" "))["model"],
            serde_json::Value::Null
        );
        assert_eq!(
            parse_iface_info(Some("1000\n"), Some("up\n"), Some("aa:bb\n"))["speed_mbit"],
            1000
        );
        assert!(parse_iface_info(Some("-1"), None, None)["speed_mbit"].is_null());
    }

    #[test]
    fn machine_info() {
        let v = parse_info(
            Some("srv1\n"),
            Some("NAME=\"Debian\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n"),
            Some("processor : 0\nmodel name\t: Intel(R) Xeon(R) CPU\n"),
        );
        assert_eq!(v["hostname"], "srv1");
        assert_eq!(v["os"], "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(v["cpu_model"], "Intel(R) Xeon(R) CPU");
    }
}
