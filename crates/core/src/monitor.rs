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
use russh_sftp::client::{fs::File, SftpSession};
use std::collections::HashMap;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Which parts of a sample are wanted. The UI asks only for what is on screen, so the files of an unused part are
/// not read at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub cpu: bool,
    pub mem: bool,
    pub load: bool,
    pub uptime: bool,
    pub net: bool,
    pub disks: bool,
}

impl Want {
    pub fn all() -> Want {
        Want {
            cpu: true,
            mem: true,
            load: true,
            uptime: true,
            net: true,
            disks: true,
        }
    }

    /// From the names the UI sends (`cpu`, `mem`, `load`, `uptime`, `net`, `disks`); unknown names are ignored.
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> Want {
        let has = |n: &str| names.iter().any(|x| x.as_ref() == n);
        Want {
            cpu: has("cpu"),
            mem: has("mem"),
            load: has("load"),
            uptime: has("uptime"),
            net: has("net"),
            disks: has("disks"),
        }
    }
}

/// How often the (large) `/proc/cpuinfo` is read again for the clock speed: every Nth sample.
const MHZ_EVERY: u32 = 10;

/// What the monitor keeps between two samples of one connection: the open `/proc` files (re-read from the start, so
/// a sample costs no open / close), the last clock speed and counters.
#[derive(Default)]
pub struct MonState {
    files: HashMap<&'static str, File>,
    mhz: Option<f64>,
    samples: u32,
    /// How many times a file had to be opened (tests: must not grow with every sample).
    pub opens: u64,
}

/// Read the whole file from the start. `None` when it cannot be read.
async fn read_all(f: &mut File, max: u64) -> Option<String> {
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).await.ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// One `/proc` file through the open handle of the previous sample (re-read from offset 0); a missing or stale handle
/// is replaced by a fresh open. Returns the text, the handle to keep and whether a file was opened.
async fn read_proc(
    sftp: &SftpSession,
    path: &str,
    cached: Option<File>,
    max: u64,
) -> (Option<String>, Option<File>, bool) {
    if let Some(mut f) = cached {
        if f.seek(SeekFrom::Start(0)).await.is_ok() {
            // an empty answer is never right for these files: the handle is stale (or the server does not
            // re-read a file from the start), so open it again
            if let Some(t) = read_all(&mut f, max).await.filter(|t| !t.is_empty()) {
                return (Some(t), Some(f), false);
            }
        }
    }
    let Ok(mut f) = sftp.open(path.to_string()).await else {
        return (None, None, true);
    };
    match read_all(&mut f, max).await {
        Some(t) => (Some(t), Some(f), true),
        None => (None, None, true),
    }
}

impl SftpConnection {
    /// Read a small text file (a `/proc` entry reports size 0, so it is read until EOF). `None` if unreadable.
    pub async fn read_small(&self, path: &str, max: u64) -> Option<String> {
        let mut f = self.light().await.open(path.to_string()).await.ok()?;
        let text = read_all(&mut f, max).await;
        let _ = f.close().await; // closes the handle and waits for the answer
        text
    }

    /// One sample of the whole server load (`with_info`: also the machine description, read once by the UI).
    pub async fn monitor_sample(&self, with_info: bool) -> Option<Value> {
        self.monitor_sample_for(with_info, Want::all()).await
    }

    /// One sample with only the wanted parts. The files stay open between samples; a sample is one read of each wanted
    /// file, all in flight at once.
    pub async fn monitor_sample_for(&self, with_info: bool, want: Want) -> Option<Value> {
        const MAX: u64 = 256 * 1024;
        let sftp = self.light().await;
        let mut st = self.mon.lock().await;
        st.samples = st.samples.wrapping_add(1);
        // the clock speed changes slowly: /proc/cpuinfo (the biggest file) is read now and then, or for the description
        let mhz_now = want.cpu && (with_info || st.mhz.is_none() || st.samples % MHZ_EVERY == 0);
        let plan: [(&'static str, bool, u64); 7] = [
            ("/proc/stat", want.cpu, MAX),
            ("/proc/meminfo", want.mem, MAX),
            ("/proc/loadavg", want.load, 4096),
            ("/proc/uptime", want.uptime, 4096),
            ("/proc/net/dev", want.net, MAX),
            ("/proc/diskstats", want.disks, MAX),
            ("/proc/cpuinfo", mhz_now || with_info, MAX),
        ];
        // all wanted files are read at the same time (a part nobody wants costs nothing)
        let mut cached: Vec<Option<File>> = plan
            .iter()
            .map(|(p, on, _)| if *on { st.files.remove(p) } else { None })
            .collect();
        let job = |i: usize, c: Option<File>| {
            let (path, on, max) = plan[i];
            async move {
                if on {
                    read_proc(sftp, path, c, max).await
                } else {
                    (None, None, false)
                }
            }
        };
        let (d0, d1, d2, d3, d4, d5, d6) = tokio::join!(
            job(0, cached[0].take()),
            job(1, cached[1].take()),
            job(2, cached[2].take()),
            job(3, cached[3].take()),
            job(4, cached[4].take()),
            job(5, cached[5].take()),
            job(6, cached[6].take()),
        );
        let mut text: HashMap<&'static str, String> = HashMap::new();
        let mut asked = 0;
        for ((path, on, _), (t, keep, opened)) in plan.iter().zip([d0, d1, d2, d3, d4, d5, d6]) {
            if !*on {
                continue;
            }
            asked += 1;
            if opened {
                st.opens += 1;
            }
            if let Some(f) = keep {
                st.files.insert(path, f);
            }
            if let Some(t) = t {
                text.insert(path, t);
            }
        }
        // nothing could be read of what was asked: not a Linux server (or the link is gone)
        if asked > 0 && text.is_empty() {
            return None;
        }
        let get = |p: &str| text.get(p).map(String::as_str);
        let cpuinfo = get("/proc/cpuinfo");
        if let Some(m) = cpuinfo.and_then(avg_mhz) {
            st.mhz = Some(m);
        }
        let mut v = parse_sample(&RawSample {
            stat: get("/proc/stat"),
            meminfo: get("/proc/meminfo"),
            loadavg: get("/proc/loadavg"),
            uptime: get("/proc/uptime"),
            netdev: get("/proc/net/dev"),
            diskstats: get("/proc/diskstats"),
            cpuinfo,
        });
        if want.cpu {
            v["cpu_mhz"] = json!(st.mhz);
        }
        let cpuinfo_text = cpuinfo.map(str::to_string);
        drop(st);
        if with_info {
            let (h, os) = tokio::join!(
                self.read_small("/proc/sys/kernel/hostname", 4096),
                self.read_small("/etc/os-release", 16 * 1024),
            );
            let mut info = parse_info(h.as_deref(), os.as_deref(), cpuinfo_text.as_deref());
            info["devices"] = self.device_info(&v).await;
            v["info"] = info;
        }
        Some(v)
    }

    /// How many `/proc` files the monitor had to open so far (tests).
    pub async fn monitor_opens(&self) -> u64 {
        self.mon.lock().await.opens
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
