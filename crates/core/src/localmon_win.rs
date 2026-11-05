//! Windows side of the local monitor: the performance counters (PDH), the same source as the Task Manager.
//! English counter names are used, so the program works in every Windows language. Counters that do not exist on
//! this computer (an old Windows without "GPU Engine", no thermal zones ...) are simply left out.
//! Rate counters (bytes/s, % busy) are integrated into cumulative counters, so the UI can treat the samples exactly
//! like the cumulative `/proc` counters of a Linux machine.
use crate::localmon::{gpu_engines, Adapter, GpuProcs, Proc};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
};
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::SystemInformation::{
    GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};

const PDH_MORE_DATA: u32 = 0x800007D2;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

type Inst = Vec<(String, f64)>;

#[derive(Default)]
struct Handles {
    cpu: Option<usize>,
    mhz: Option<usize>,
    disk_busy: Option<usize>,
    disk_rb: Option<usize>,
    disk_wb: Option<usize>,
    disk_r: Option<usize>,
    disk_w: Option<usize>,
    disk_sr: Option<usize>,
    disk_sw: Option<usize>,
    disk_q: Option<usize>,
    net_rx: Option<usize>,
    net_tx: Option<usize>,
    net_bw: Option<usize>,
    gpu_eng: Option<usize>,
    gpu_mem: Option<usize>,
    gpu_proc_mem: Option<usize>,
    thermal: Option<usize>,
    proc_cpu: Option<usize>,
    proc_id: Option<usize>,
    proc_ws: Option<usize>,
}

pub struct WinCounters {
    /// PDH handles are pointers; they are kept as numbers so that the monitor can live inside the (Send + Sync) service.
    query: usize,
    h: Handles,
    last: Option<Instant>,
    cores: Vec<[f64; 2]>,
    total: [f64; 2],
    disks: BTreeMap<String, [f64; 8]>,
    ifaces: BTreeMap<String, [f64; 2]>,
    bandwidth: BTreeMap<String, f64>,
    adapters: Vec<Adapter>,
    gpu_procs: GpuProcs,
    procs: Vec<Proc>,
    temp: Option<f64>,
}

impl WinCounters {
    pub fn new() -> Self {
        let mut qp: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: plain PDH call with a valid out pointer.
        unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut qp) };
        let query = qp as usize;
        let add = |path: &str| -> Option<usize> {
            let mut h: *mut core::ffi::c_void = std::ptr::null_mut();
            let p = wide(path);
            // SAFETY: the path is a NUL-terminated UTF-16 string, h is a valid out pointer.
            let r = unsafe {
                PdhAddEnglishCounterW(query as *mut core::ffi::c_void, p.as_ptr(), 0, &mut h)
            };
            (r == 0 && !h.is_null()).then_some(h as usize)
        };
        let h = Handles {
            cpu: add("\\Processor(*)\\% Processor Time"),
            mhz: add("\\Processor Information(_Total)\\Processor Frequency"),
            disk_busy: add("\\PhysicalDisk(*)\\% Disk Time"),
            disk_rb: add("\\PhysicalDisk(*)\\Disk Read Bytes/sec"),
            disk_wb: add("\\PhysicalDisk(*)\\Disk Write Bytes/sec"),
            disk_r: add("\\PhysicalDisk(*)\\Disk Reads/sec"),
            disk_w: add("\\PhysicalDisk(*)\\Disk Writes/sec"),
            disk_sr: add("\\PhysicalDisk(*)\\Avg. Disk sec/Read"),
            disk_sw: add("\\PhysicalDisk(*)\\Avg. Disk sec/Write"),
            disk_q: add("\\PhysicalDisk(*)\\Current Disk Queue Length"),
            net_rx: add("\\Network Interface(*)\\Bytes Received/sec"),
            net_tx: add("\\Network Interface(*)\\Bytes Sent/sec"),
            net_bw: add("\\Network Interface(*)\\Current Bandwidth"),
            gpu_eng: add("\\GPU Engine(*)\\Utilization Percentage"),
            gpu_mem: add("\\GPU Adapter Memory(*)\\Dedicated Usage"),
            gpu_proc_mem: add("\\GPU Process Memory(*)\\Dedicated Usage"),
            thermal: add("\\Thermal Zone Information(*)\\Temperature"),
            proc_cpu: add("\\Process(*)\\% Processor Time"),
            proc_id: add("\\Process(*)\\ID Process"),
            proc_ws: add("\\Process(*)\\Working Set - Private"),
        };
        // SAFETY: valid query handle.
        unsafe { PdhCollectQueryData(query as *mut core::ffi::c_void) };
        Self {
            query,
            h,
            last: None,
            cores: Vec::new(),
            total: [0.0; 2],
            disks: BTreeMap::new(),
            ifaces: BTreeMap::new(),
            bandwidth: BTreeMap::new(),
            adapters: Vec::new(),
            gpu_procs: HashMap::new(),
            procs: Vec::new(),
            temp: None,
        }
    }

    /// All instances of a wildcard counter with their current (formatted) values; invalid ones are skipped.
    fn read(&self, h: Option<usize>) -> Inst {
        let Some(h) = h else { return Vec::new() };
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: the first call only asks for the size.
        let r = unsafe {
            PdhGetFormattedCounterArrayW(
                h as *mut core::ffi::c_void,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if r != PDH_MORE_DATA || size == 0 {
            return Vec::new();
        }
        // u64 elements keep the buffer aligned for the items
        let mut buf = vec![0u64; (size as usize).div_ceil(8) + 1];
        let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        // SAFETY: the buffer is at least `size` bytes.
        let r = unsafe {
            PdhGetFormattedCounterArrayW(
                h as *mut core::ffi::c_void,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                items,
            )
        };
        if r != 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            // SAFETY: PDH filled `count` items; the names point into the same buffer and are NUL-terminated.
            let it = unsafe { &*items.add(i) };
            if it.FmtValue.CStatus > 1 || it.szName.is_null() {
                continue;
            }
            let mut len = 0usize;
            // SAFETY: see above.
            unsafe {
                while *it.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                out.push((name, it.FmtValue.Anonymous.doubleValue));
            }
        }
        out
    }

    pub fn sample(&mut self, with_info: bool) -> Value {
        // SAFETY: valid query handle.
        unsafe { PdhCollectQueryData(self.query as *mut core::ffi::c_void) };
        let now = Instant::now();
        let dt = self
            .last
            .map_or(0.0, |l| now.duration_since(l).as_secs_f64())
            .clamp(0.0, 60.0);
        self.last = Some(now);
        let dt_ms = dt * 1000.0;

        // processor: % busy per logical processor -> cumulative [busy, total] milliseconds
        let cpu = self.read(self.h.cpu);
        let mut per: Vec<(usize, f64)> = cpu
            .iter()
            .filter_map(|(n, v)| n.parse::<usize>().ok().map(|i| (i, *v)))
            .collect();
        per.sort_by_key(|p| p.0);
        if self.cores.len() != per.len() {
            self.cores = vec![[0.0; 2]; per.len()];
        }
        for (i, (_, v)) in per.iter().enumerate() {
            self.cores[i][0] += v.clamp(0.0, 100.0) / 100.0 * dt_ms;
            self.cores[i][1] += dt_ms;
        }
        if let Some((_, v)) = cpu.iter().find(|(n, _)| n == "_Total") {
            self.total[0] += v.clamp(0.0, 100.0) / 100.0 * dt_ms;
            self.total[1] += dt_ms;
        }
        let mhz = self.read(self.h.mhz).first().map(|p| p.1);

        // memory
        // SAFETY: zeroed plain structs with their size field set.
        let mut ms: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let mut pi: PERFORMANCE_INFORMATION = unsafe { std::mem::zeroed() };
        pi.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
        // SAFETY: valid pointers to initialised structs.
        let (ok_m, ok_p) = unsafe {
            (
                GlobalMemoryStatusEx(&mut ms) != 0,
                GetPerformanceInfo(&mut pi, pi.cb) != 0,
            )
        };
        let mem = ok_m.then(|| {
            let cache = if ok_p { pi.SystemCache as u64 * pi.PageSize as u64 } else { 0 };
            json!({
                "total": ms.ullTotalPhys, "available": ms.ullAvailPhys, "free": ms.ullAvailPhys.saturating_sub(cache.min(ms.ullAvailPhys)),
                "cached": cache.min(ms.ullTotalPhys),
                "swap_total": ms.ullTotalPageFile.saturating_sub(ms.ullTotalPhys),
                "swap_free": ms.ullAvailPageFile.saturating_sub(ms.ullAvailPhys),
            })
        });

        // network: bytes/s -> cumulative bytes per interface
        let (rx, tx) = (self.read(self.h.net_rx), self.read(self.h.net_tx));
        let skip = |n: &str| {
            n.to_lowercase().contains("loopback")
                || n.to_lowercase().contains("isatap")
                || n.to_lowercase().contains("teredo")
        };
        for (n, v) in rx.iter().filter(|(n, _)| !skip(n)) {
            self.ifaces.entry(n.clone()).or_insert([0.0; 2])[0] += v.max(0.0) * dt;
        }
        for (n, v) in tx.iter().filter(|(n, _)| !skip(n)) {
            self.ifaces.entry(n.clone()).or_insert([0.0; 2])[1] += v.max(0.0) * dt;
        }
        for (n, v) in self.read(self.h.net_bw) {
            self.bandwidth.insert(n, v);
        }
        let ifaces: Vec<Value> = self
            .ifaces
            .iter()
            .map(|(n, c)| json!({ "name": n, "rx": c[0] as u64, "tx": c[1] as u64 }))
            .collect();
        let net = json!({ "rx": self.ifaces.values().map(|c| c[0]).sum::<f64>() as u64, "tx": self.ifaces.values().map(|c| c[1]).sum::<f64>() as u64 });

        // disks: rates -> cumulative [reads, sectors read, ms reading, writes, sectors written, ms writing, ms busy, in flight]
        let rd = |h: Option<usize>| -> HashMap<String, f64> { self.read(h).into_iter().collect() };
        let (busy, rb, wb, r, w, sr, sw, q) = (
            rd(self.h.disk_busy),
            rd(self.h.disk_rb),
            rd(self.h.disk_wb),
            rd(self.h.disk_r),
            rd(self.h.disk_w),
            rd(self.h.disk_sr),
            rd(self.h.disk_sw),
            rd(self.h.disk_q),
        );
        let names: Vec<String> = busy
            .keys()
            .chain(rb.keys())
            .filter(|n| *n != "_Total")
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for n in &names {
            let g = |m: &HashMap<String, f64>| m.get(n).copied().unwrap_or(0.0).max(0.0);
            let c = self.disks.entry(n.clone()).or_insert([0.0; 8]);
            c[0] += g(&r) * dt;
            c[1] += g(&rb) * dt / 512.0;
            c[2] += g(&sr) * 1000.0 * g(&r) * dt;
            c[3] += g(&w) * dt;
            c[4] += g(&wb) * dt / 512.0;
            c[5] += g(&sw) * 1000.0 * g(&w) * dt;
            c[6] += g(&busy).min(100.0) / 100.0 * dt_ms;
            c[7] = g(&q);
        }
        let disks: Vec<Value> = self
            .disks
            .iter()
            .map(
                |(n, c)| json!({ "name": n, "c": c.iter().map(|x| *x as u64).collect::<Vec<_>>() }),
            )
            .collect();

        // video cards and what the processes use of them
        let eng = self.read(self.h.gpu_eng);
        let by_adapter = gpu_engines(&eng);
        let gmem: Inst = self.read(self.h.gpu_mem);
        let adapter_key = |n: &str| n.split("_phys_").next().unwrap_or(n).to_string();
        let mut adapters: BTreeMap<String, (BTreeMap<String, f64>, Option<u64>)> = BTreeMap::new();
        for (a, e) in by_adapter {
            adapters.entry(a).or_default().0 = e;
        }
        for (n, v) in &gmem {
            adapters.entry(adapter_key(n)).or_default().1 = Some(*v as u64);
        }
        self.adapters = adapters.into_iter().map(|(a, (e, m))| (a, e, m)).collect();
        // per process: the busiest engine, and the dedicated memory
        let mut per_pid: HashMap<u32, HashMap<String, f64>> = HashMap::new();
        for (n, v) in &eng {
            let pid = n
                .strip_prefix("pid_")
                .and_then(|r| r.split('_').next())
                .and_then(|p| p.parse::<u32>().ok());
            if let Some(pid) = pid {
                let key = n.split("_luid_").nth(1).unwrap_or("").to_string();
                *per_pid.entry(pid).or_default().entry(key).or_insert(0.0) += v;
            }
        }
        let mut gp: HashMap<u32, (f64, u64)> = HashMap::new();
        for (pid, m) in per_pid {
            gp.insert(pid, (m.values().cloned().fold(0.0, f64::max).min(100.0), 0));
        }
        for (n, v) in self.read(self.h.gpu_proc_mem) {
            if let Some(pid) = n
                .strip_prefix("pid_")
                .and_then(|r| r.split('_').next())
                .and_then(|p| p.parse::<u32>().ok())
            {
                gp.entry(pid).or_insert((0.0, 0)).1 += v as u64;
            }
        }
        self.gpu_procs = gp;

        // processes
        let ncores = self.cores.len().max(1) as f64;
        let ids: HashMap<String, f64> = self.read(self.h.proc_id).into_iter().collect();
        let ws: HashMap<String, f64> = self.read(self.h.proc_ws).into_iter().collect();
        self.procs = self
            .read(self.h.proc_cpu)
            .into_iter()
            .filter(|(n, _)| n != "_Total" && n != "Idle")
            .filter_map(|(n, v)| {
                let pid = *ids.get(&n)? as u32;
                let name = n.rsplit_once('#').map_or(n.clone(), |(a, _)| a.to_string());
                Some(Proc {
                    pid,
                    name,
                    cpu: (v / ncores).clamp(0.0, 100.0),
                    mem: ws.get(&n).copied().unwrap_or(0.0) as u64,
                    gpu: None,
                    vram: None,
                })
            })
            .collect();

        // processor temperature (ACPI thermal zones, Kelvin) when Windows reports it
        self.temp = self
            .read(self.h.thermal)
            .into_iter()
            .map(|(_, k)| k - 273.15)
            .filter(|c| *c > 10.0 && *c < 120.0)
            .fold(None, |m: Option<f64>, c| Some(m.map_or(c, |x| x.max(c))));

        // SAFETY: plain call.
        let uptime = unsafe { GetTickCount64() } as f64 / 1000.0;
        let mut v = json!({
            "cpu": [self.total[0] as u64, self.total[1] as u64],
            "cores": self.cores.iter().map(|c| json!([c[0] as u64, c[1] as u64])).collect::<Vec<_>>(),
            "mem": mem, "load": Value::Null, "uptime": uptime, "net": net, "disks": disks, "ifaces": ifaces, "cpu_mhz": mhz,
        });
        if with_info {
            let mut info = crate::localmon::windows_info(
                &reg_query(
                    "HKLM\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0",
                    "ProcessorNameString",
                ),
                &reg_query(
                    "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
                    "ProductName",
                ),
                std::env::var("COMPUTERNAME").ok().as_deref(),
            );
            let ifs: serde_json::Map<String, Value> = self.bandwidth.iter().map(|(n, bw)| (n.clone(), json!({ "speed_mbit": (*bw > 0.0).then(|| (bw / 1_000_000.0) as i64), "state": "up", "mac": Value::Null }))).collect();
            let dks: serde_json::Map<String, Value> = names
                .iter()
                .map(|n| {
                    (
                        n.clone(),
                        json!({ "size": Value::Null, "kind": "", "model": Value::Null }),
                    )
                })
                .collect();
            info["devices"] = json!({ "disks": dks, "ifaces": ifs });
            v["info"] = info;
        }
        v
    }

    /// (adapters with engine loads and used memory, per-process video load and memory)
    pub fn gpu_state(&self) -> (Vec<Adapter>, GpuProcs) {
        (self.adapters.clone(), self.gpu_procs.clone())
    }

    pub fn processes(&self) -> Vec<Proc> {
        self.procs.clone()
    }

    pub fn cpu_temp(&self) -> Option<f64> {
        self.temp
    }
}

/// `reg query <key> /v <value>` -> the data of the value ("" when missing). A local, read-only registry query.
fn reg_query(key: &str, value: &str) -> String {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(["query", key, "/v", value])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000);
    cmd.output()
        .ok()
        .map(|o| crate::localmon::parse_reg_value(&String::from_utf8_lossy(&o.stdout), value))
        .unwrap_or_default()
}
