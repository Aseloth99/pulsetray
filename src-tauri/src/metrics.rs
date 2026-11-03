use crate::battery::{self, BatteryDetail};
use crate::diskhealth::{self, DiskHealth};
use crate::gpusensors::{self, GpuSensors};
use crate::hardware::{self, GpuStatic};
use crate::pdh::Pdh;
use crate::procs::ProcTable;
use crate::usage::UsageStore;
use crate::wifi::{self, WifiInfo};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use sysinfo::{
    CpuRefreshKind, DiskKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub fs: String,
    /// "SSD", "HDD" or "".
    pub kind: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    pub present: bool,
    pub percent: u8,
    pub charging: bool,
    pub plugged: bool,
    /// Remaining seconds, if Windows can estimate it.
    pub remaining_secs: Option<u32>,
    pub detail: Option<BatteryDetail>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuNow {
    pub luid: String,
    pub name: String,
    pub usage: f32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub shared_used: u64,
    pub shared_total: u64,
    /// Clocks / temperature / fan / power, whatever the driver reports.
    pub sensors: GpuSensors,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetAdapterNow {
    pub name: String,
    pub ip: String,
    pub mac: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProcInfo {
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
    /// Disk I/O in bytes per second.
    pub disk: f64,
    pub gpu: f32,
    pub count: u32,
    /// Every process of this name (the list groups them), for "open file location" / "end task".
    pub pids: Vec<u32>,
}

/// How many apps each "top apps" list carries: the tile shows 5 (3 in the classic view) and more when its
/// column has room to spare.
const TOP_APPS: usize = 10;

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    /// When this sample was taken (ms since the epoch): the charts are drawn against it.
    pub ts_ms: u64,
    // CPU
    pub cpu_name: String,
    pub cpu_cores: usize,
    pub cpu_physical: usize,
    pub cpu_usage: f32,
    pub cpu_per_core: Vec<f32>,
    /// Clock of every logical processor (MHz), same order as `cpu_per_core`; empty while the popup is closed.
    pub cpu_core_mhz: Vec<u32>,
    pub cpu_freq_mhz: u64,
    pub cpu_temp_c: Option<f32>,
    pub cpu_power_w: Option<f32>,
    /// Where the CPU temperature / power came from: "acpi" / "meter" (Windows), "" = none.
    pub cpu_temp_src: &'static str,
    pub cpu_power_src: &'static str,
    // Memory
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub mem_cached: u64,
    pub commit_used: u64,
    pub commit_limit: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    // Network (rates in bytes/s, totals in bytes)
    pub net_down: f64,
    pub net_up: f64,
    pub net_boot_down: u64,
    pub net_boot_up: u64,
    pub net_session_down: u64,
    pub net_session_up: u64,
    pub net_today_down: u64,
    pub net_today_up: u64,
    pub net_adapter: Option<NetAdapterNow>,
    // Disk
    pub disk_read: f64,
    pub disk_write: f64,
    pub disk_active: f32,
    pub disks: Vec<DiskInfo>,
    /// Temperature / wear per physical drive (see diskhealth.rs); refreshed every 30 s while on screen.
    pub disk_health: Vec<DiskHealth>,
    // GPU / battery / Wi-Fi / processes / system
    pub gpus: Vec<GpuNow>,
    pub battery: Battery,
    pub wifi: Option<WifiInfo>,
    /// Is there a Wi-Fi adapter at all? Known even while the Wi-Fi tile is off and `wifi` is not read.
    pub wifi_present: bool,
    /// Frames per second of the app in front; only while the mini gauge shows FPS (see fps.rs).
    pub fps: Option<crate::fps::FpsReading>,
    pub top_cpu: Vec<ProcInfo>,
    pub top_mem: Vec<ProcInfo>,
    pub top_disk: Vec<ProcInfo>,
    pub top_gpu: Vec<ProcInfo>,
    /// The top lists were measured in this sample (they cover the time since the previous measurement).
    #[serde(skip)]
    pub procs_sampled: bool,
    /// ... and the per-app GPU load in them is current (the GPU counters only run while something shows them).
    #[serde(skip)]
    pub procs_gpu_fresh: bool,
    pub process_count: u32,
    pub thread_count: u32,
    pub handle_count: u32,
    pub uptime_secs: u64,
}

/// What the UI currently shows. Everything that isn't on screen is simply not measured:
/// the process list, GPU counters and battery IOCTLs are the expensive parts.
#[derive(Clone, Copy, Default)]
pub struct Features {
    pub processes: bool,
    pub gpu: bool,
    pub battery: bool,
    pub wifi: bool,
    pub ping: bool,
    /// An alert is switched on: drive space, commit and drive health are also read while the popup is closed.
    pub alerts: bool,
}

/// Slower-changing / costlier values, refreshed on their own cadence and reused in between.
#[derive(Default)]
struct Cache {
    disks: Vec<DiskInfo>,
    disk_health: Vec<DiskHealth>,
    disk_health_read: bool,
    disk_read: f64,
    disk_write: f64,
    disk_active: f32,
    cpu_freq: u64,
    /// Task Manager's CPU load (total and per logical processor), when the counters exist.
    cpu_busy: Option<f32>,
    core_busy: Vec<f32>,
    core_mhz: Vec<u32>,
    /// Page file (total, in use), bytes.
    pagefile: Option<(u64, u64)>,
    cpu_temp: Option<f32>,
    cpu_power: Option<f32>,
    cpu_temp_src: &'static str,
    cpu_power_src: &'static str,
    gpus: Vec<GpuNow>,
    gpu_pids: HashMap<u32, f32>,
    battery_detail: Option<BatteryDetail>,
    wifi: Option<WifiInfo>,
    wifi_present: bool,
    wifi_probed: bool,
    perf: (u32, u32, u32, u64, u64, u64),
    top_cpu: Vec<ProcInfo>,
    top_mem: Vec<ProcInfo>,
    top_disk: Vec<ProcInfo>,
    top_gpu: Vec<ProcInfo>,
    procs_sampled: bool,
    procs_gpu_fresh: bool,
    /// When the per-process GPU loads were last read.
    gpu_pids_at: Option<Instant>,
}

pub struct Sampler {
    sys: System,
    networks: Networks,
    disks: Disks,
    pdh: Option<Pdh>,
    gpus: Vec<GpuStatic>,
    usage: UsageStore,
    session: (u64, u64),
    last: Instant,
    last_save: Instant,
    /// When the disk I/O counters were last refreshed (they only refresh while the popup is open).
    last_disk: Instant,
    /// Per-process CPU / memory / disk (one system call, see procs.rs).
    procs: ProcTable,
    /// Every NVIDIA adapter name DXGI reported (NVML devices are matched against it).
    nvidia_names: Vec<String>,
    /// The adapter LUIDs the GPU counters listed on the previous read (see `gpu_list_stale`).
    counter_luids: Option<HashSet<String>>,
    /// Earliest moment the adapter list may be read again, and how long to wait after a read that changed nothing.
    gpu_relist_at: Instant,
    gpu_relist_wait: Duration,
    cpu_temp_filter: StaticSensor,
    tick: u64,
    cache: Cache,
    physical_cores: usize,
}

/// ACPI thermal zones on some desktop boards are constants (a fixed 27.8 °C, say). A real CPU sensor never
/// reports the exact same value for minutes while the PC is in use, so after a long unchanged run the
/// reading is dropped for the rest of the session instead of showing a fake temperature.
struct StaticSensor {
    last: f32,
    run: u32,
    dead: bool,
}

impl StaticSensor {
    /// Visible seconds of an identical reading before it is judged to be a constant.
    const LIMIT: u32 = 300;

    fn new() -> Self {
        Self { last: f32::NAN, run: 0, dead: false }
    }

    fn filter(&mut self, v: Option<f32>) -> Option<f32> {
        if self.dead {
            return None;
        }
        let v = v?;
        if v == self.last {
            self.run += 1;
        } else {
            self.last = v;
            self.run = 0;
        }
        if self.run >= Self::LIMIT {
            self.dead = true;
            return None;
        }
        Some(v)
    }
}

/// The adapter list (LUIDs) is read from DXGI once at start-up, but a card gets a new LUID when its driver is
/// installed or reset while the PC runs, and adapters come and go (virtual displays, an eGPU). The counters
/// then name LUIDs the list does not know, and the card reads 0 % / 0 B until the program is restarted.
/// The list is stale when the counters' LUID set changed since the last read (`seen` = None on the first
/// one) or when a listed card has no counters at all (`all_listed` = false).
fn gpu_list_stale(seen: Option<&HashSet<String>>, now: &HashSet<String>, all_listed: bool) -> bool {
    seen.is_some_and(|s| s != now) || !all_listed
}

/// Does every adapter of the list have counters (under its own LUID or an alias)?
fn all_have_counters(gpus: &[GpuStatic], ids: &HashSet<String>) -> bool {
    gpus.iter().all(|g| ids.contains(&g.luid) || g.alias_luids.iter().any(|a| ids.contains(a)))
}

fn nvidia_names_of(gpus: &[GpuStatic]) -> Vec<String> {
    gpus.iter().filter(|g| g.name.to_lowercase().contains("nvidia")).map(|g| g.name.clone()).collect()
}

/// A re-read that finds the same list is repeated less and less often (an adapter that never gets counters
/// must not keep a sleeping laptop dGPU busy); one that finds a new list is checked again after the minimum.
const GPU_RELIST_MIN: Duration = Duration::from_secs(30);
const GPU_RELIST_MAX: Duration = Duration::from_secs(600);

/// Read the adapter list from DXGI again (see `gpu_list_stale`): the new list (None if it is the same, or empty
/// because the driver is still coming up) and how long to wait before the next re-read.
fn relist_gpus(current: &[GpuStatic], wait: Duration) -> (Option<Vec<GpuStatic>>, Duration) {
    let fresh = hardware::dxgi_gpus();
    let same = fresh.iter().map(|g| &g.luid).eq(current.iter().map(|g| &g.luid));
    let wait = if same { (wait * 2).min(GPU_RELIST_MAX) } else { GPU_RELIST_MIN };
    (if same || fresh.is_empty() { None } else { Some(fresh) }, wait)
}

/// Reading a GPU's sensors (NVML, D3DKMT) can wake a sleeping laptop dGPU and keep it awake, which costs
/// battery. An adapter with no load and nothing allocated is idle, so leave it alone until it has work.
fn worth_probing(usage: f32, dedicated_used: u64) -> bool {
    usage > 0.0 || dedicated_used > 0
}

/// Virtual / non-physical interfaces would double count traffic (WSL, VMs, VPN taps, Bluetooth PAN).
fn is_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "loopback", "vethernet", "hyper-v", "virtualbox", "vmware", "wsl", "bluetooth", "tap-", "pseudo",
        "miniport", "isatap", "teredo", "nordlynx", "wireguard", "wintun", "openvpn", "tailscale",
        "zerotier", "anyconnect", "warp", "vpn",
    ]
    .iter()
    .any(|k| n.contains(k))
}

impl Sampler {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let gpus = hardware::dxgi_gpus();
        let nvidia_names = nvidia_names_of(&gpus);
        Self {
            sys,
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything()),
            pdh: Pdh::new(),
            gpus,
            usage: UsageStore::load(data_dir),
            session: (0, 0),
            last: Instant::now(),
            last_save: Instant::now(),
            last_disk: Instant::now(),
            procs: ProcTable::default(),
            nvidia_names,
            counter_luids: None,
            gpu_relist_at: Instant::now(),
            gpu_relist_wait: GPU_RELIST_MIN,
            cpu_temp_filter: StaticSensor::new(),
            tick: 0,
            cache: Cache::default(),
            physical_cores: System::physical_core_count().unwrap_or(0),
        }
    }

    /// `visible` = popup is open. Hidden, only what the charts' history and the tray tooltip need is measured
    /// (no process list, Wi-Fi or system counters).
    pub fn sample(&mut self, visible: bool, f: Features) -> Metrics {
        self.tick += 1;
        let tick = self.tick;
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64().max(0.001);
        self.last = now;

        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        // Re-listing the interfaces is the costly part; counters alone are enough between re-lists.
        self.networks.refresh(tick % 30 == 1);

        // --- network (always: tooltip + daily totals)
        let (mut rx, mut tx, mut boot_rx, mut boot_tx) = (0u64, 0u64, 0u64, 0u64);
        let mut best: Option<(u64, NetAdapterNow)> = None;
        for (name, data) in self.networks.iter() {
            if is_virtual(name) {
                continue;
            }
            rx += data.received();
            tx += data.transmitted();
            boot_rx += data.total_received();
            boot_tx += data.total_transmitted();
            if visible {
                if let Some(ip) = data.ip_networks().iter().find(|n| n.addr.is_ipv4()) {
                    // Lifetime traffic keeps the choice stable (per-tick traffic would flip between near-ties).
                    let score = data.total_received().saturating_add(data.total_transmitted()).max(1);
                    if best.as_ref().is_none_or(|(s, _)| score > *s) {
                        best = Some((
                            score,
                            NetAdapterNow {
                                name: name.clone(),
                                ip: ip.addr.to_string(),
                                mac: data.mac_address().to_string(),
                            },
                        ));
                    }
                }
            }
        }
        self.session.0 += rx;
        self.session.1 += tx;
        self.usage.add(rx, tx);
        if self.last_save.elapsed() > Duration::from_secs(30) {
            self.usage.save();
            self.last_save = Instant::now();
        }

        self.sample_details(tick, f, visible);
        let procs_sampled = std::mem::take(&mut self.cache.procs_sampled);

        let cpus = self.sys.cpus();
        let c = &self.cache;
        let per_core = if c.core_busy.len() == cpus.len() {
            c.core_busy.clone()
        } else {
            cpus.iter().map(|c| c.cpu_usage()).collect()
        };
        let core_mhz = if c.core_mhz.len() == per_core.len() { c.core_mhz.clone() } else { Vec::new() };
        let (swap_total, swap_used) = c.pagefile.unwrap_or_else(|| (self.sys.total_swap(), self.sys.used_swap()));
        Metrics {
            ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            cpu_name: cpus.first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
            cpu_cores: cpus.len(),
            cpu_physical: if self.physical_cores > 0 { self.physical_cores } else { cpus.len() },
            cpu_usage: c.cpu_busy.unwrap_or_else(|| self.sys.global_cpu_usage()),
            cpu_per_core: per_core,
            cpu_core_mhz: core_mhz,
            cpu_freq_mhz: c.cpu_freq,
            cpu_temp_c: c.cpu_temp,
            cpu_power_w: c.cpu_power,
            cpu_temp_src: c.cpu_temp_src,
            cpu_power_src: c.cpu_power_src,
            mem_total: self.sys.total_memory(),
            mem_used: self.sys.used_memory(),
            mem_available: self.sys.available_memory(),
            mem_cached: c.perf.5,
            commit_used: c.perf.3,
            commit_limit: c.perf.4,
            swap_total,
            swap_used,
            net_down: rx as f64 / dt,
            net_up: tx as f64 / dt,
            net_boot_down: boot_rx,
            net_boot_up: boot_tx,
            net_session_down: self.session.0,
            net_session_up: self.session.1,
            net_today_down: self.usage.today.rx,
            net_today_up: self.usage.today.tx,
            net_adapter: best.map(|(_, a)| a),
            disk_read: c.disk_read,
            disk_write: c.disk_write,
            disk_active: c.disk_active,
            disks: c.disks.clone(),
            disk_health: c.disk_health.clone(),
            gpus: c.gpus.clone(),
            battery: read_battery(c.battery_detail.clone()),
            wifi: c.wifi.clone(),
            wifi_present: c.wifi_present,
            fps: None, // filled in by the sampler loop when the gauge shows it
            top_cpu: c.top_cpu.clone(),
            top_mem: c.top_mem.clone(),
            top_disk: c.top_disk.clone(),
            top_gpu: c.top_gpu.clone(),
            procs_sampled,
            procs_gpu_fresh: procs_sampled && c.procs_gpu_fresh,
            process_count: c.perf.0,
            thread_count: c.perf.1,
            handle_count: c.perf.2,
            uptime_secs: System::uptime(),
        }
    }

    /// Visible: everything on screen. Hidden (one tick every 2 s): only what the charts' history needs.
    fn sample_details(&mut self, tick: u64, f: Features, visible: bool) {
        // The disk counters span the time since the last refresh. Divide by that span, and show nothing for a
        // long gap (a first reading would otherwise be minutes of I/O squeezed into a few seconds).
        let disk_dt = self.last_disk.elapsed().as_secs_f64().max(0.001);
        self.last_disk = Instant::now();
        let disk_stale = disk_dt > 5.0;
        // Disk I/O every tick; capacity and file systems every 15 s on screen (every minute in the background when
        // the "drive almost full" alert is on).
        let mut kind = DiskRefreshKind::nothing().with_io_usage();
        if (visible && tick % 15 == 1) || (!visible && f.alerts && tick % 30 == 1) || self.cache.disks.is_empty() {
            kind = kind.with_storage();
        }
        self.disks.refresh_specifics(true, kind);
        let (mut rd, mut wr) = (0u64, 0u64);
        let mut disks: Vec<DiskInfo> = Vec::new();
        for d in self.disks.list() {
            let u = d.usage();
            rd += u.read_bytes;
            wr += u.written_bytes;
            let mount = d.mount_point().to_string_lossy().to_string();
            let total = d.total_space();
            if total == 0 || disks.iter().any(|x| x.mount == mount) {
                continue;
            }
            disks.push(DiskInfo {
                name: d.name().to_string_lossy().to_string(),
                mount,
                fs: d.file_system().to_string_lossy().to_string(),
                kind: match d.kind() {
                    DiskKind::SSD => "SSD",
                    DiskKind::HDD => "HDD",
                    _ => "",
                }
                .to_string(),
                total,
                used: total.saturating_sub(d.available_space()),
            });
        }
        let c = &mut self.cache;
        c.disks = disks;
        c.disk_read = if disk_stale { 0.0 } else { rd as f64 / disk_dt };
        c.disk_write = if disk_stale { 0.0 } else { wr as f64 / disk_dt };

        if let Some(p) = &self.pdh {
            p.collect();
            c.cpu_busy = p.cpu_busy();
            c.core_busy = p.core_busy();
            c.disk_active = p.disk_active();
            if let Some(mhz) = p.cpu_mhz() {
                c.cpu_freq = mhz;
            }
            c.core_mhz = if visible { p.core_mhz() } else { Vec::new() };
        }
        let (win_temp, win_power) = match &self.pdh {
            Some(p) => (self.cpu_temp_filter.filter(p.cpu_temp_c()), p.cpu_power_w()),
            None => (None, None),
        };
        // A running Afterburner / HWiNFO knows the real die temperature and package power (AMD gives Windows
        // neither); the ACPI zone and the Intel energy meter are only the fallback.
        let ext = crate::extsensors::cpu();
        (c.cpu_temp, c.cpu_temp_src) = match ext.temp {
            Some((t, src)) => (Some(t), src),
            None => (win_temp, if win_temp.is_some() { "acpi" } else { "" }),
        };
        (c.cpu_power, c.cpu_power_src) = match ext.power {
            Some((w, src)) => (Some(w), src),
            None => (win_power, if win_power.is_some() { "meter" } else { "" }),
        };
        // Every tick: the memory chart's page-file line is part of the history.
        c.pagefile = crate::procs::pagefile();
        if c.cpu_freq == 0 {
            let cpus = self.sys.cpus();
            c.cpu_freq = if cpus.is_empty() {
                0
            } else {
                cpus.iter().map(|c| c.frequency()).sum::<u64>() / cpus.len() as u64
            };
        }

        // Hidden ticks are 2 s apart already; on screen the slow parts run every other (1 s) tick.
        let every2 = !visible || tick % 2 == 0;

        // GPU counters (hundreds of PDH instances): only if something shows them (per-app GPU: on screen only).
        if (f.gpu || (visible && f.processes)) && every2 {
            if let Some(p) = &self.pdh {
                p.collect_gpu();
                let (counters, pids) = p.gpus();
                if f.gpu {
                    let ids: HashSet<String> = counters.keys().cloned().collect();
                    let stale = gpu_list_stale(self.counter_luids.as_ref(), &ids, all_have_counters(&self.gpus, &ids));
                    if !stale {
                        self.counter_luids = Some(ids);
                    } else if Instant::now() >= self.gpu_relist_at {
                        let (fresh, wait) = relist_gpus(&self.gpus, self.gpu_relist_wait);
                        self.gpu_relist_wait = wait;
                        self.gpu_relist_at = Instant::now() + wait;
                        if let Some(fresh) = fresh {
                            self.nvidia_names = nvidia_names_of(&fresh);
                            self.gpus = fresh;
                        }
                        self.counter_luids = Some(ids);
                    }
                }
                c.gpu_pids = pids;
                c.gpu_pids_at = Some(Instant::now());
                c.gpus = self
                    .gpus
                    .iter()
                    .map(|g| {
                        let mut k = counters.get(&g.luid).copied().unwrap_or_default();
                        for a in &g.alias_luids {
                            if let Some(o) = counters.get(a) {
                                k.usage = k.usage.max(o.usage);
                                k.dedicated_used = k.dedicated_used.max(o.dedicated_used);
                                k.shared_used = k.shared_used.max(o.shared_used);
                            }
                        }
                        GpuNow {
                            luid: g.luid.clone(),
                            name: g.name.clone(),
                            usage: k.usage,
                            vram_used: k.dedicated_used,
                            vram_total: g.vram_total,
                            shared_used: k.shared_used,
                            shared_total: g.shared_total,
                            sensors: if f.gpu && worth_probing(k.usage, k.dedicated_used) {
                                gpusensors::read(&g.luid, &g.name, &self.nvidia_names)
                            } else {
                                GpuSensors::default()
                            },
                        }
                    })
                    .collect();
            }
        } else if c.gpus.is_empty() && !self.gpus.is_empty() {
            // The static part is known right away so the tile can appear before the first counter read.
            c.gpus = self
                .gpus
                .iter()
                .map(|g| GpuNow {
                    luid: g.luid.clone(),
                    name: g.name.clone(),
                    vram_total: g.vram_total,
                    shared_total: g.shared_total,
                    ..Default::default()
                })
                .collect();
        }

        if f.battery && every2 {
            c.battery_detail = battery::read();
        }
        // Process list: every 3rd tick, also in the tray (3 s apart on screen, 6 s hidden). Besides the "top apps"
        // tile it tells the charts who caused a peak, and the alerts which app hogs the CPU. One system call
        // (procs.rs); the first reading only sets the baseline.
        if tick % 3 == 0 {
            let cores = self.sys.cpus().len().max(1);
            if let Some(list) = self.procs.sample(cores) {
                // Per-app GPU loads are only as fresh as the last GPU counter read.
                let gpu_fresh = c.gpu_pids_at.is_some_and(|t| t.elapsed() < Duration::from_secs(5));
                // Process times are busy-time shares of the whole CPU, the same measure as the CPU tile.
                let mut agg: HashMap<String, ProcInfo> = HashMap::new();
                for p in list {
                    let e = agg.entry(p.name.clone()).or_insert_with(|| ProcInfo { name: p.name.clone(), ..Default::default() });
                    e.cpu = (e.cpu + p.cpu).min(100.0);
                    e.mem += p.mem;
                    e.disk += p.disk;
                    if gpu_fresh {
                        e.gpu += c.gpu_pids.get(&p.pid).copied().unwrap_or(0.0);
                    }
                    e.count += 1;
                    e.pids.push(p.pid);
                }
                let all: Vec<ProcInfo> = agg.into_values().collect();
                let top = |key: fn(&ProcInfo) -> f64| {
                    let mut v = all.clone();
                    v.sort_by(|a, b| key(b).total_cmp(&key(a)));
                    v.truncate(TOP_APPS);
                    v
                };
                c.top_cpu = top(|p| p.cpu as f64);
                c.top_mem = top(|p| p.mem as f64);
                c.top_disk = top(|p| p.disk);
                c.top_gpu = top(|p| p.gpu as f64);
                c.procs_sampled = true;
                c.procs_gpu_fresh = gpu_fresh;
            }
        }
        // Commit charge and drive health: on screen, or for the alerts in the background (every 10 s / minute).
        if !visible && f.alerts {
            if tick % 5 == 1 || c.perf.0 == 0 {
                c.perf = perf_info();
            }
            if tick % 30 == 3 || !c.disk_health_read {
                c.disk_health = diskhealth::read_all();
                c.disk_health_read = true;
            }
        }
        if !visible {
            return;
        }
        // Drive temperature / wear changes slowly: once on the first visible tick, then every 30 s.
        if tick % 30 == 3 || !c.disk_health_read {
            c.disk_health = diskhealth::read_all();
            c.disk_health_read = true;
        }
        if f.wifi && tick % 2 == 0 {
            c.wifi = wifi::read();
            c.wifi_present = c.wifi.is_some();
            c.wifi_probed = true;
        } else if !f.wifi && (!c.wifi_probed || tick % 30 == 3) {
            // The tile is off, so `wifi` is not read, but Settings must still know whether the hardware is there.
            c.wifi_present = wifi::read().is_some();
            c.wifi_probed = true;
        }
        if tick % 5 == 1 || c.perf.0 == 0 {
            c.perf = perf_info();
        }
    }
}

#[cfg(windows)]
fn perf_info() -> (u32, u32, u32, u64, u64, u64) {
    use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
    let mut p = PERFORMANCE_INFORMATION::default();
    let size = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    if unsafe { GetPerformanceInfo(&mut p, size) }.is_err() {
        return (0, 0, 0, 0, 0, 0);
    }
    let page = p.PageSize as u64;
    (
        p.ProcessCount,
        p.ThreadCount,
        p.HandleCount,
        p.CommitTotal as u64 * page,
        p.CommitLimit as u64 * page,
        p.SystemCache as u64 * page,
    )
}

#[cfg(not(windows))]
fn perf_info() -> (u32, u32, u32, u64, u64, u64) {
    (0, 0, 0, 0, 0, 0)
}

/// Some drivers report the rate as a magnitude only. Charging is positive and running on the battery is
/// negative by definition, so force the sign from the system power state (plugged-in idle: keep as is).
fn normalize_rate(rate: i32, charging: bool, plugged: bool) -> i32 {
    if charging {
        rate.saturating_abs()
    } else if !plugged {
        -rate.saturating_abs()
    } else {
        rate
    }
}

#[cfg(windows)]
fn read_battery(detail: Option<BatteryDetail>) -> Battery {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return Battery::default();
    }
    // BatteryFlag: 128 = no system battery, 255 = unknown.
    let present = s.BatteryFlag != 128 && s.BatteryFlag != 255 && s.BatteryLifePercent != 255;
    let (charging, plugged) = (s.BatteryFlag & 8 != 0, s.ACLineStatus == 1);
    let mut detail = detail;
    if let Some(d) = detail.as_mut() {
        d.rate_mw = normalize_rate(d.rate_mw, charging, plugged);
    }
    Battery {
        present,
        percent: if present { s.BatteryLifePercent.min(100) } else { 0 },
        charging,
        plugged,
        remaining_secs: (s.BatteryLifeTime != u32::MAX).then_some(s.BatteryLifeTime),
        detail: if present { detail } else { None },
    }
}

#[cfg(not(windows))]
fn read_battery(_detail: Option<BatteryDetail>) -> Battery {
    Battery::default()
}

#[cfg(test)]
mod bench {
    use super::*;

    #[test]
    #[ignore = "reads this machine's processes: run with --ignored --nocapture"]
    fn live_top_apps_are_recorded_while_hidden() {
        let mut s = Sampler::new(None);
        let mut h = crate::history::History::default();
        let mut last = 0;
        for _ in 0..7 {
            let m = s.sample(false, Features::default());
            last = m.ts_ms;
            h.push(&m);
            std::thread::sleep(Duration::from_millis(400));
        }
        let cpu = h.tops_at(last, "cpu").expect("a process measurement covers the newest sample");
        println!("cpu {:?}\nmem {:?}", cpu.apps, h.tops_at(last, "mem").map(|d| d.apps));
        assert!(!cpu.apps.is_empty() && cpu.apps.len() <= 3);
        assert!(h.tops_at(last, "gpu").is_none(), "no GPU counters ran: no GPU names");
    }

    #[test]
    #[ignore = "needs a GPU and the Windows counters: run with --ignored --nocapture"]
    fn live_a_card_that_got_a_new_luid_is_found_again() {
        let mut s = Sampler::new(None);
        let f = Features { gpu: true, ..Features::default() };
        for _ in 0..3 {
            s.sample(false, f);
            std::thread::sleep(Duration::from_millis(300));
        }
        let real: Vec<String> = s.gpus.iter().map(|g| g.luid.clone()).collect();
        assert!(!real.is_empty(), "this machine lists no GPU");
        // What a driver update does to a running program: the saved LUIDs no longer exist.
        for (i, g) in s.gpus.iter_mut().enumerate() {
            g.luid = format!("luid_0x00000000_0x0bad{i:04x}");
        }
        s.gpu_relist_at = Instant::now();
        let m = s.sample(false, f);
        assert_eq!(s.gpus.iter().map(|g| g.luid.clone()).collect::<Vec<_>>(), real, "the list is read again");
        for g in &m.gpus {
            println!("{}  usage {:.0} %  vram {} MB / {} MB", g.name, g.usage, g.vram_used >> 20, g.vram_total >> 20);
        }
        // Nothing changed now: the next reads must not read the list again before the back-off is over.
        let at = s.gpu_relist_at;
        s.sample(false, f);
        assert_eq!(s.gpu_relist_at, at);
    }

    #[test]
    #[ignore = "timing benchmark: run with --ignored --nocapture"]
    fn sample_cost() {
        let mut s = Sampler::new(None);
        let all = Features { processes: true, gpu: true, battery: true, wifi: true, ping: true, alerts: true };
        let none = Features::default();
        for (name, vis, f) in [
            ("hidden/no tiles", false, none),
            ("hidden/all", false, all),
            ("visible/no tiles", true, none),
            ("visible/all", true, all),
        ] {
            let (mut worst, mut sum) = (Duration::ZERO, Duration::ZERO);
            for _ in 0..12 {
                let t = Instant::now();
                s.sample(vis, f);
                let d = t.elapsed();
                worst = worst.max(d);
                sum += d;
                std::thread::sleep(Duration::from_millis(100));
            }
            let avg = sum / 12;
            println!("{name:<18} avg {:>6.2} ms   worst {:>6.2} ms", avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
        }
        let time = |name: &str, f: &mut dyn FnMut()| {
            let t = Instant::now();
            for _ in 0..10 {
                f();
                std::thread::sleep(Duration::from_millis(20));
            }
            let total = t.elapsed();
            println!("  part {name:<22} ~{:>6.2} ms (incl. sleeps)", total.as_secs_f64() * 100.0 - 20.0);
        };
        time("refresh_cpu_usage", &mut || s.sys.refresh_cpu_usage());
        time("procs.sample", &mut || {
            let _ = s.procs.sample(12);
        });
        time("refresh_memory", &mut || s.sys.refresh_memory());
        time("networks.refresh(false)", &mut || s.networks.refresh(false));
        time("System::uptime", &mut || {
            let _ = System::uptime();
        });
        time("cpus()+brand+per_core", &mut || {
            let c = s.sys.cpus();
            let _: Vec<f32> = c.iter().map(|x| x.cpu_usage()).collect();
        });
        time("read_battery", &mut || {
            let _ = read_battery(None);
        });
        time("perf_info", &mut || {
            let _ = perf_info();
        });
        let t = Instant::now();
        let r = crate::ping::ping_all(&["gateway".into(), "1.1.1.1".into(), "8.8.8.8".into()]);
        println!("ping x3 (parallel)  {:>6.1} ms  {:?}", t.elapsed().as_secs_f64() * 1e3, r.iter().map(|x| x.rtt).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn a_constant_temperature_sensor_is_dropped_but_a_live_one_is_kept() {
        let mut s = StaticSensor::new();
        for i in 0..StaticSensor::LIMIT * 2 {
            // wobbles by a tenth of a degree: a real sensor
            let v = 41.0 + (i % 3) as f32 * 0.1;
            assert_eq!(s.filter(Some(v)), Some(v));
        }
        let mut s = StaticSensor::new();
        let mut shown = 0;
        for _ in 0..StaticSensor::LIMIT * 2 {
            if s.filter(Some(27.85)).is_some() {
                shown += 1;
            }
        }
        assert_eq!(shown, StaticSensor::LIMIT, "shown until the run is long enough, never again afterwards");
        assert_eq!(s.filter(Some(55.0)), None, "once judged constant it stays off for the session");
        assert_eq!(StaticSensor::new().filter(None), None);
    }

    #[test]
    fn idle_gpus_are_not_probed() {
        assert!(!worth_probing(0.0, 0), "no load, nothing allocated: a sleeping dGPU must be left alone");
        assert!(worth_probing(3.5, 0));
        assert!(worth_probing(0.0, 120 * 1024 * 1024));
    }

    #[test]
    fn a_card_that_comes_back_under_a_new_luid_makes_the_list_stale() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<HashSet<String>>();
        let igpu = "luid_0x00000000_0x00014f9a";
        let dgpu = "luid_0x00000000_0x00014c2b";
        let gpus = vec![
            GpuStatic { luid: igpu.into(), ..Default::default() },
            GpuStatic { luid: dgpu.into(), ..Default::default() },
        ];
        let before = ids(&[igpu, dgpu, "luid_0x00000000_0x00014fbf"]); // the last one: a software adapter DXGI leaves out
        assert!(!gpu_list_stale(None, &before, all_have_counters(&gpus, &before)), "first read, everything matches");
        assert!(!gpu_list_stale(Some(&before), &before, all_have_counters(&gpus, &before)), "nothing changed");

        // The driver was installed: the card is listed under a new LUID, the integrated one is untouched.
        let after = ids(&[igpu, "luid_0x00000000_0x0fe650bb", "luid_0x00000000_0x00014fbf"]);
        assert!(!all_have_counters(&gpus, &after), "the saved dGPU has no counters any more");
        assert!(gpu_list_stale(Some(&before), &after, all_have_counters(&gpus, &after)));
        assert!(gpu_list_stale(None, &after, all_have_counters(&gpus, &after)), "started with an outdated list");

        // A new adapter appears (eGPU, virtual display) while the known ones keep their counters.
        let plus = ids(&[igpu, dgpu, "luid_0x00000000_0x0fe650bb"]);
        assert!(all_have_counters(&gpus, &plus));
        assert!(gpu_list_stale(Some(&before), &plus, true));

        // The same card under two LUIDs is covered through its alias.
        let aliased = vec![GpuStatic { luid: dgpu.into(), alias_luids: vec!["luid_0x00000000_0x0000aaaa".into()], ..Default::default() }];
        assert!(all_have_counters(&aliased, &ids(&["luid_0x00000000_0x0000aaaa"])));
        assert!(all_have_counters(&[], &ids(&[])), "no adapters: nothing to be out of date");
    }

    #[test]
    fn nvidia_cards_are_picked_by_name() {
        let gpus = vec![
            GpuStatic { name: "NVIDIA GeForce RTX 4070".into(), ..Default::default() },
            GpuStatic { name: "AMD Radeon(TM) Graphics".into(), ..Default::default() },
        ];
        assert_eq!(nvidia_names_of(&gpus), vec!["NVIDIA GeForce RTX 4070".to_string()]);
    }

    #[test]
    fn rate_sign_follows_the_power_state() {
        assert_eq!(normalize_rate(-21_700, true, true), 21_700, "charging is positive even if the driver says negative");
        assert_eq!(normalize_rate(8_500, false, false), -8_500, "on battery is negative even if the driver says positive");
        assert_eq!(normalize_rate(-8_500, false, false), -8_500);
        assert_eq!(normalize_rate(300, false, true), 300, "plugged in and idle: leave it alone");
        assert_eq!(normalize_rate(i32::MIN, false, false), -i32::MAX, "no overflow on the most negative value");
    }

    #[test]
    fn virtual_adapters_are_not_counted() {
        for n in ["vEthernet (WSL)", "NordLynx", "Cloudflare WARP", "Hyper-V Virtual Ethernet Adapter", "Bluetooth Device (Personal Area Network)", "OpenVPN TAP-Windows6", "Tailscale", "Loopback Pseudo-Interface 1"] {
            assert!(is_virtual(n), "{n}");
        }
        for n in ["Ethernet", "Wi-Fi", "Realtek USB GbE Family Controller", "MediaTek Wi-Fi 6 MT7921 Wireless LAN Card", "Intel(R) Ethernet Connection (7) I219-V"] {
            assert!(!is_virtual(n), "{n}");
        }
    }
}
