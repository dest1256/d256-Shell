//! Load monitor of THIS computer: the twin of the server monitor (`monitor.rs`), giving samples of the same shape
//! (CPU, memory, disks, network as cumulative counters the UI turns into rates) plus what only a local machine can
//! offer: the video card (3D / Compute-CUDA / Copy / Video engines, memory, temperature, power), the processor
//! temperature and the busiest processes.
//!
//! Linux reads `/proc` and `/sys` (the same parser as the server monitor). Windows reads the performance counters
//! (PDH, the data of the Task Manager) in `localmon_win.rs`. The video card of NVIDIA is also asked through
//! `nvidia-smi` when it is installed (a local, read-only query; `D256SHELL_NVIDIA_SMI` can point to it).
//! Nothing here touches the network.
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// An adapter of the Windows counters: id, load per engine type, used video memory.
pub type Adapter = (String, BTreeMap<String, f64>, Option<u64>);
/// Per process: busiest video engine (%) and dedicated video memory (bytes).
pub type GpuProcs = HashMap<u32, (f64, u64)>;

/// One video card.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Gpu {
    pub id: String,
    pub name: String,
    /// Overall utilization (nvidia-smi / sysfs), 0..100.
    pub util: Option<f64>,
    /// Utilization per engine type: "3D", "Cuda", "Compute" (DirectCompute), "Copy", "VideoDecode", "VideoEncode" ... (Windows counters).
    pub engines: BTreeMap<String, f64>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub temp: Option<f64>,
    pub power: Option<f64>,
    pub power_limit: Option<f64>,
    pub fan: Option<f64>,
    pub clock: Option<f64>,
}

/// A busy process.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// Share of the whole processor, 0..100.
    pub cpu: f64,
    /// Working set / resident memory, bytes.
    pub mem: u64,
    /// Share of the busiest video engine, 0..100.
    pub gpu: Option<f64>,
    pub vram: Option<u64>,
}

// ------------------------------------------------------------------ nvidia-smi

/// The fields asked from nvidia-smi (the order matters for `parse_nvidia_csv`).
pub const NVIDIA_FIELDS: &str = "index,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw,power.limit,fan.speed,clocks.gr";

fn num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t.starts_with('[') || t.eq_ignore_ascii_case("n/a") {
        return None;
    }
    t.parse().ok()
}

/// `nvidia-smi --query-gpu=<NVIDIA_FIELDS> --format=csv,noheader,nounits`. Memory is reported in MiB.
pub fn parse_nvidia_csv(out: &str) -> Vec<Gpu> {
    let mib = |v: Option<f64>| v.map(|m| (m * 1024.0 * 1024.0) as u64);
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 10 || f[1].is_empty() {
                return None;
            }
            Some(Gpu {
                id: f[0].to_string(),
                name: f[1].to_string(),
                util: num(f[2]),
                vram_used: mib(num(f[3])),
                vram_total: mib(num(f[4])),
                temp: num(f[5]),
                power: num(f[6]),
                power_limit: num(f[7]),
                fan: num(f[8]),
                clock: num(f[9]),
                engines: BTreeMap::new(),
            })
        })
        .collect()
}

/// `--query-compute-apps=pid,used_memory --format=csv,noheader,nounits` -> bytes per pid.
pub fn parse_nvidia_apps(out: &str) -> HashMap<u32, u64> {
    let mut m = HashMap::new();
    for l in out.lines() {
        let f: Vec<&str> = l.split(',').map(str::trim).collect();
        if f.len() >= 2 {
            if let (Ok(pid), Some(mb)) = (f[0].parse::<u32>(), num(f[1])) {
                *m.entry(pid).or_insert(0) += (mb * 1024.0 * 1024.0) as u64;
            }
        }
    }
    m
}

fn run_with_timeout(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (program, args): (String, Vec<String>) = (
        program.to_string(),
        args.iter().map(|s| s.to_string()).collect(),
    );
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let _ = tx.send(
            cmd.output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned()),
        );
    });
    rx.recv_timeout(limit).ok().flatten()
}

/// Data of a value from the output of  ("    Name    REG_SZ    data").
pub fn parse_reg_value(out: &str, value: &str) -> String {
    out.lines()
        .find(|l| l.trim_start().starts_with(value))
        .and_then(|l| l.split("REG_").nth(1))
        .and_then(|r| r.split_once(char::is_whitespace))
        .map(|(_, d)| d.trim().to_string())
        .unwrap_or_default()
}

/// The one-off description of a Windows machine for the monitor.
pub fn windows_info(cpu_model: &str, os_name: &str, host: Option<&str>) -> Value {
    let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    json!({ "hostname": host, "os": some(os_name), "cpu_model": some(cpu_model) })
}

// ------------------------------------------------------------------ merging and ranking (platform independent)

/// Engine counters of Windows ("GPU Engine(pid_12_luid_0x0_0x1_phys_0_eng_3_engtype_3D)", value %) -> per adapter
/// (`luid_0x.._0x..`) and engine type the busiest single engine, where the load of all processes on one engine is summed.
pub fn gpu_engines(instances: &[(String, f64)]) -> BTreeMap<String, BTreeMap<String, f64>> {
    // (adapter, engine index, type) -> sum over processes
    let mut sums: BTreeMap<(String, String, String), f64> = BTreeMap::new();
    for (name, v) in instances {
        let Some(luid) = name.find("luid_").map(|i| &name[i..]) else {
            continue;
        };
        let adapter: String = luid.split("_phys_").next().unwrap_or(luid).to_string();
        let eng = luid
            .split("_eng_")
            .nth(1)
            .and_then(|r| r.split('_').next())
            .unwrap_or("0")
            .to_string();
        let ty = name.rsplit("engtype_").next().unwrap_or("").to_string();
        if ty.is_empty() || ty == *name {
            continue;
        }
        *sums.entry((adapter, eng, ty)).or_insert(0.0) += v;
    }
    let mut out: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for ((adapter, _eng, ty), v) in sums {
        // "Compute_0" and friends are one family (DirectCompute); CUDA is its own engine type "Cuda", which is what
        // the Task Manager calls "Cuda": the two must not be mixed (a browser or a game computes too)
        let ty = ty.split('_').next().unwrap_or(&ty).to_string();
        let ty = if ty.eq_ignore_ascii_case("cuda") {
            "Cuda".to_string()
        } else {
            ty
        };
        let e = out.entry(adapter).or_default().entry(ty).or_insert(0.0);
        *e = e.max(v.min(100.0));
    }
    out
}

/// Pair the adapters of the Windows counters with the cards reported by nvidia-smi: by the closeness of the used video
/// memory (the only thing both report). Unpaired adapters stay as "GPU n".
pub fn merge_gpus(
    adapters: &[(String, BTreeMap<String, f64>, Option<u64>)],
    smi: Vec<Gpu>,
) -> Vec<Gpu> {
    let mut used = vec![false; adapters.len()];
    let mut out: Vec<Gpu> = Vec::new();
    for g in smi {
        let mut best: Option<usize> = None;
        for (i, a) in adapters.iter().enumerate() {
            if used[i] {
                continue;
            }
            let dist = |x: Option<u64>| match (x, g.vram_used) {
                (Some(a), Some(b)) => a.abs_diff(b),
                _ => u64::MAX / 2,
            };
            if best.is_none_or(|b| dist(a.2) < dist(adapters[b].2)) {
                best = Some(i);
            }
        }
        let mut g = g;
        if let Some(i) = best {
            used[i] = true;
            g.engines = adapters[i].1.clone();
        }
        out.push(g);
    }
    for (i, a) in adapters.iter().enumerate() {
        if !used[i] {
            out.push(Gpu {
                id: format!("{}", out.len()),
                name: format!("GPU {}", out.len()),
                engines: a.1.clone(),
                vram_used: a.2,
                ..Default::default()
            });
        }
    }
    out
}

/// The busiest processes first: by video load, then processor load. Idle ones are dropped.
pub fn top_processes(mut all: Vec<Proc>, n: usize) -> Vec<Proc> {
    all.retain(|p| p.cpu >= 0.05 || p.gpu.unwrap_or(0.0) >= 0.5 || p.vram.unwrap_or(0) > 0);
    all.sort_by(|a, b| {
        let key = |p: &Proc| (p.gpu.unwrap_or(0.0) * 4.0).max(p.cpu);
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.mem.cmp(&a.mem))
    });
    all.truncate(n);
    all
}

// ------------------------------------------------------------------ the monitor

pub struct LocalMonitor {
    smi_cache: Option<(Instant, Vec<Gpu>, HashMap<u32, u64>)>,
    smi_missing_until: Option<Instant>,
    #[cfg(not(windows))]
    procs_prev: Option<(Instant, HashMap<u32, u64>)>,
    #[cfg(windows)]
    win: Option<crate::localmon_win::WinCounters>,
}

impl Default for LocalMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalMonitor {
    pub fn new() -> Self {
        Self {
            smi_cache: None,
            smi_missing_until: None,
            #[cfg(not(windows))]
            procs_prev: None,
            #[cfg(windows)]
            win: None,
        }
    }

    fn smi_program() -> String {
        std::env::var("D256SHELL_NVIDIA_SMI")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "nvidia-smi".into())
    }

    /// Cards and per-process memory from nvidia-smi, at most once a second; a missing program is not retried for a minute.
    fn nvidia(&mut self) -> (Vec<Gpu>, HashMap<u32, u64>) {
        if let Some((at, g, a)) = &self.smi_cache {
            if at.elapsed() < Duration::from_millis(900) {
                return (g.clone(), a.clone());
            }
        }
        if self.smi_missing_until.is_some_and(|t| Instant::now() < t) {
            return (Vec::new(), HashMap::new());
        }
        let prog = Self::smi_program();
        let q = format!("--query-gpu={NVIDIA_FIELDS}");
        let Some(out) = run_with_timeout(
            &prog,
            &[&q, "--format=csv,noheader,nounits"],
            Duration::from_secs(4),
        ) else {
            self.smi_missing_until = Some(Instant::now() + Duration::from_secs(60));
            self.smi_cache = None;
            return (Vec::new(), HashMap::new());
        };
        let apps = run_with_timeout(
            &prog,
            &[
                "--query-compute-apps=pid,used_memory",
                "--format=csv,noheader,nounits",
            ],
            Duration::from_secs(4),
        )
        .map(|o| parse_nvidia_apps(&o))
        .unwrap_or_default();
        let gpus = parse_nvidia_csv(&out);
        self.smi_cache = Some((Instant::now(), gpus.clone(), apps.clone()));
        (gpus, apps)
    }

    /// One sample in the shape of the server monitor, plus `gpus`, `cpu_temp` and `procs`.
    pub fn sample(&mut self, with_info: bool) -> Value {
        let (smi, apps) = self.nvidia();
        #[cfg(windows)]
        {
            let w = self
                .win
                .get_or_insert_with(crate::localmon_win::WinCounters::new);
            let mut v = w.sample(with_info);
            let (adapters, procs_gpu) = w.gpu_state();
            let gpus = merge_gpus(&adapters, smi);
            let mut procs = w.processes();
            for p in &mut procs {
                if let Some((g, vr)) = procs_gpu.get(&p.pid) {
                    p.gpu = Some(*g);
                    p.vram = Some(*vr);
                } else if let Some(vr) = apps.get(&p.pid) {
                    p.vram = Some(*vr);
                }
            }
            v["gpus"] = json!(gpus);
            v["cpu_temp"] = json!(w.cpu_temp());
            v["procs"] = json!(top_processes(procs, 15));
            v
        }
        #[cfg(not(windows))]
        {
            let mut v = linux::sample(with_info);
            let mut gpus = smi;
            gpus.extend(linux::amd_gpus(gpus.len()));
            v["gpus"] = json!(gpus);
            v["cpu_temp"] = json!(linux::cpu_temp());
            let ncores = v["cores"].as_array().map_or(1, |c| c.len().max(1));
            let mut procs = linux::processes(&mut self.procs_prev, ncores);
            for p in &mut procs {
                p.vram = apps.get(&p.pid).copied();
            }
            v["procs"] = json!(top_processes(procs, 15));
            v
        }
    }
}

#[cfg(not(windows))]
pub(crate) mod linux {
    use super::*;
    use crate::monitor::{parse_info, parse_sample, RawSample};
    use std::fs;

    fn read(p: &str) -> Option<String> {
        fs::read_to_string(p).ok()
    }

    pub fn sample(with_info: bool) -> Value {
        let (stat, mem, load, up, net, disks, cpuinfo) = (
            read("/proc/stat"),
            read("/proc/meminfo"),
            read("/proc/loadavg"),
            read("/proc/uptime"),
            read("/proc/net/dev"),
            read("/proc/diskstats"),
            read("/proc/cpuinfo"),
        );
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
            let mut info = parse_info(
                read("/proc/sys/kernel/hostname").as_deref(),
                read("/etc/os-release").as_deref(),
                cpuinfo.as_deref(),
            );
            info["devices"] = devices(&v);
            v["info"] = info;
        }
        v
    }

    fn devices(sample: &Value) -> Value {
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
        let safe = |n: &String| {
            !n.is_empty()
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.:@".contains(c))
                && !n.starts_with('.')
        };
        let mut disks = serde_json::Map::new();
        for n in names("disks").iter().filter(|n| safe(n)) {
            let (s, r, m) = (
                read(&format!("/sys/block/{n}/size")),
                read(&format!("/sys/block/{n}/queue/rotational")),
                read(&format!("/sys/block/{n}/device/model")),
            );
            disks.insert(
                n.clone(),
                crate::monitor::parse_disk_info(s.as_deref(), r.as_deref(), m.as_deref()),
            );
        }
        let mut ifaces = serde_json::Map::new();
        for n in names("ifaces").iter().filter(|n| safe(n)) {
            let (sp, st, mac) = (
                read(&format!("/sys/class/net/{n}/speed")),
                read(&format!("/sys/class/net/{n}/operstate")),
                read(&format!("/sys/class/net/{n}/address")),
            );
            ifaces.insert(
                n.clone(),
                crate::monitor::parse_iface_info(sp.as_deref(), st.as_deref(), mac.as_deref()),
            );
        }
        json!({ "disks": disks, "ifaces": ifaces })
    }

    /// Cards of AMD (and others that expose it) through sysfs: `gpu_busy_percent`, VRAM and the hwmon sensors.
    pub fn amd_gpus(start: usize) -> Vec<Gpu> {
        let Ok(rd) = fs::read_dir("/sys/class/drm") else {
            return Vec::new();
        };
        let mut cards: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("card") && n[4..].bytes().all(|b| b.is_ascii_digit()))
            .collect();
        cards.sort();
        let mut out = Vec::new();
        for c in cards {
            let dev = format!("/sys/class/drm/{c}/device");
            let Some(busy) =
                read(&format!("{dev}/gpu_busy_percent")).and_then(|s| s.trim().parse::<f64>().ok())
            else {
                continue;
            };
            let b =
                |f: &str| read(&format!("{dev}/{f}")).and_then(|s| s.trim().parse::<u64>().ok());
            let hw = fs::read_dir(format!("{dev}/hwmon"))
                .ok()
                .and_then(|mut r| r.next())
                .and_then(|e| e.ok())
                .map(|e| e.path());
            let hv = |f: &str| {
                hw.as_ref()
                    .and_then(|p| read(&p.join(f).to_string_lossy()))
                    .and_then(|s| s.trim().parse::<f64>().ok())
            };
            out.push(Gpu {
                id: (start + out.len()).to_string(),
                name: read(&format!("{dev}/product_name"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| format!("GPU {}", start + out.len())),
                util: Some(busy),
                vram_used: b("mem_info_vram_used"),
                vram_total: b("mem_info_vram_total"),
                temp: hv("temp1_input").map(|m| m / 1000.0),
                power: hv("power1_average").map(|u| u / 1_000_000.0),
                ..Default::default()
            });
        }
        out
    }

    /// Package temperature of the processor, °C, when a sensor of a known driver exists.
    pub fn cpu_temp() -> Option<f64> {
        let mut best: Option<f64> = None;
        if let Ok(rd) = fs::read_dir("/sys/class/hwmon") {
            for e in rd.flatten() {
                let p = e.path();
                let name = read(&p.join("name").to_string_lossy()).unwrap_or_default();
                if ![
                    "coretemp",
                    "k10temp",
                    "zenpower",
                    "cpu_thermal",
                    "cpu-thermal",
                ]
                .contains(&name.trim())
                {
                    continue;
                }
                for i in 1..=16 {
                    if let Some(m) = read(&p.join(format!("temp{i}_input")).to_string_lossy())
                        .and_then(|s| s.trim().parse::<f64>().ok())
                    {
                        best = Some(best.map_or(m / 1000.0, |b: f64| b.max(m / 1000.0)));
                    }
                }
            }
        }
        if best.is_none() {
            if let Ok(rd) = fs::read_dir("/sys/class/thermal") {
                for e in rd.flatten() {
                    let p = e.path();
                    let ty = read(&p.join("type").to_string_lossy()).unwrap_or_default();
                    if ty.contains("x86_pkg_temp") || ty.contains("cpu") {
                        if let Some(m) = read(&p.join("temp").to_string_lossy())
                            .and_then(|s| s.trim().parse::<f64>().ok())
                        {
                            best = Some(best.map_or(m / 1000.0, |b: f64| b.max(m / 1000.0)));
                        }
                    }
                }
            }
        }
        best.filter(|t| *t > 0.0 && *t < 150.0)
    }

    /// `/proc/<pid>/stat` -> (name, cpu ticks). The name sits in parentheses and may contain spaces and parentheses.
    pub fn parse_stat(text: &str) -> Option<(String, u64)> {
        let a = text.find('(')?;
        let b = text.rfind(')')?;
        let name = text[a + 1..b].to_string();
        let f: Vec<&str> = text[b + 1..].split_whitespace().collect();
        // after ")": state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt utime stime ...
        let ut: u64 = f.get(11)?.parse().ok()?;
        let st: u64 = f.get(12)?.parse().ok()?;
        Some((name, ut + st))
    }

    pub fn processes(prev: &mut Option<(Instant, HashMap<u32, u64>)>, ncores: usize) -> Vec<Proc> {
        let now = Instant::now();
        let mut ticks: HashMap<u32, u64> = HashMap::new();
        let mut rows: Vec<(u32, String, u64)> = Vec::new();
        let Ok(rd) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Some((name, t)) = read(&format!("/proc/{pid}/stat")).and_then(|s| parse_stat(&s))
            else {
                continue;
            };
            let rss = read(&format!("/proc/{pid}/statm"))
                .and_then(|s| {
                    s.split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok())
                })
                .unwrap_or(0)
                * 4096;
            ticks.insert(pid, t);
            rows.push((pid, name, rss));
        }
        let mut out = Vec::new();
        if let Some((at, old)) = prev.as_ref() {
            let dt = now.duration_since(*at).as_secs_f64().max(0.05);
            for (pid, name, rss) in rows {
                let cpu = match (old.get(&pid), ticks.get(&pid)) {
                    (Some(a), Some(b)) if b >= a => {
                        (*b - *a) as f64 / 100.0 / dt / ncores as f64 * 100.0
                    }
                    _ => 0.0,
                };
                out.push(Proc {
                    pid,
                    name,
                    cpu: cpu.clamp(0.0, 100.0),
                    mem: rss,
                    gpu: None,
                    vram: None,
                });
            }
        }
        *prev = Some((now, ticks));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_csv_is_parsed_and_unsupported_fields_become_none() {
        let out = "0, NVIDIA GeForce RTX 4090, 92, 21914, 24564, 71, 312.45, 450.00, 64, 2535\n1, Tesla T4, 3, 120, 15360, 40, [N/A], 70.00, [Not Supported], 585\n\ngarbage\n";
        let v = parse_nvidia_csv(out);
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].name.as_str(), v[0].util, v[0].temp),
            ("NVIDIA GeForce RTX 4090", Some(92.0), Some(71.0))
        );
        assert_eq!(v[0].vram_total, Some(24564 * 1024 * 1024));
        assert_eq!(
            (v[0].power, v[0].power_limit, v[0].fan, v[0].clock),
            (Some(312.45), Some(450.0), Some(64.0), Some(2535.0))
        );
        assert_eq!((v[1].power, v[1].fan), (None, None));
    }

    #[test]
    fn compute_apps_memory_is_summed_per_pid() {
        let m = parse_nvidia_apps("1234, 20800\n1234, 100\n99, [N/A]\nbad\n");
        assert_eq!(m.get(&1234), Some(&(20900 * 1024 * 1024)));
        assert!(!m.contains_key(&99));
    }

    #[test]
    fn engine_counters_are_summed_per_engine_and_the_busiest_of_a_type_wins() {
        let inst = |n: &str, v: f64| (n.to_string(), v);
        let e = gpu_engines(&[
            inst(
                "pid_100_luid_0x00000000_0x0000A1_phys_0_eng_0_engtype_3D",
                60.0,
            ),
            inst(
                "pid_200_luid_0x00000000_0x0000A1_phys_0_eng_0_engtype_3D",
                30.0,
            ),
            inst(
                "pid_100_luid_0x00000000_0x0000A1_phys_0_eng_1_engtype_3D",
                10.0,
            ),
            inst(
                "pid_100_luid_0x00000000_0x0000A1_phys_0_eng_2_engtype_Compute_0",
                88.0,
            ),
            inst(
                "pid_100_luid_0x00000000_0x0000A1_phys_0_eng_3_engtype_Copy",
                4.0,
            ),
            inst(
                "pid_300_luid_0x00000000_0x0000B2_phys_0_eng_0_engtype_3D",
                5.0,
            ),
            inst(
                "pid_100_luid_0x00000000_0x0000A1_phys_0_eng_4_engtype_Cuda",
                0.0,
            ),
            inst("garbage", 50.0),
        ]);
        let a = &e["luid_0x00000000_0x0000A1"];
        assert_eq!(
            (a["3D"], a["Compute"], a["Copy"]),
            (90.0, 88.0, 4.0),
            "two processes on one engine add up"
        );
        assert_eq!(
            a["Cuda"], 0.0,
            "CUDA is not the same as the Compute engines"
        );
        assert_eq!(e["luid_0x00000000_0x0000B2"]["3D"], 5.0);
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn adapters_are_paired_with_nvidia_cards_by_used_memory() {
        let gib = 1024u64 * 1024 * 1024;
        let eng = |v: f64| BTreeMap::from([("3D".to_string(), v)]);
        let adapters = vec![
            ("igpu".to_string(), eng(1.0), Some(gib / 2)),
            ("dgpu".to_string(), eng(90.0), Some(20 * gib)),
        ];
        let smi = vec![Gpu {
            id: "0".into(),
            name: "RTX".into(),
            vram_used: Some(20 * gib + 1000),
            ..Default::default()
        }];
        let m = merge_gpus(&adapters, smi);
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].name.as_str(), m[0].engines["3D"]),
            ("RTX", 90.0),
            "the loaded adapter belongs to the card"
        );
        assert_eq!((m[1].name.as_str(), m[1].engines["3D"]), ("GPU 1", 1.0));
        // without nvidia-smi the adapters stand alone
        let alone = merge_gpus(&adapters, Vec::new());
        assert_eq!(alone.len(), 2);
    }

    #[test]
    fn busiest_processes_come_first_and_idle_ones_are_dropped() {
        let p = |pid, cpu, gpu: Option<f64>, vram| Proc {
            pid,
            name: format!("p{pid}"),
            cpu,
            mem: 1,
            gpu,
            vram,
        };
        let t = top_processes(
            vec![
                p(1, 0.0, None, None),
                p(2, 30.0, None, None),
                p(3, 5.0, Some(91.0), Some(1 << 30)),
                p(4, 60.0, Some(0.0), None),
            ],
            3,
        );
        assert_eq!(t.iter().map(|x| x.pid).collect::<Vec<_>>(), [3, 4, 2]);
        assert_eq!(top_processes(vec![p(9, 0.0, None, None)], 5).len(), 0);
    }

    #[test]
    fn registry_values_are_read_from_reg_query_output() {
        let out = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\x\r\n    ProcessorNameString    REG_SZ    AMD Ryzen 9 7950X 16-Core Processor    \r\n\r\n";
        assert_eq!(
            parse_reg_value(out, "ProcessorNameString"),
            "AMD Ryzen 9 7950X 16-Core Processor"
        );
        assert_eq!(parse_reg_value(out, "Nope"), "");
        assert_eq!(windows_info("cpu", "", Some("PC"))["os"], Value::Null);
    }

    #[cfg(not(windows))]
    #[test]
    fn proc_stat_with_spaces_and_parentheses_in_the_name() {
        let s = "4242 (my (odd) name) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 1 0 12345 1000 10 18446744073709551615";
        assert_eq!(
            linux::parse_stat(s),
            Some(("my (odd) name".to_string(), 200))
        );
        assert_eq!(linux::parse_stat("garbage"), None);
    }
}
