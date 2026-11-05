#![allow(unsafe_op_in_unsafe_fn)]
//! Windows Performance Data Helper (PDH) counters: per-adapter GPU load / memory and disk active time.
//! These are the same counters Task Manager uses, and they work without admin rights.

use std::collections::HashMap;
use windows::core::{w, PCWSTR};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

pub struct Pdh {
    /// Cheap counters (disk time, CPU clock).
    query: PDH_HQUERY,
    /// GPU counters expand to hundreds of instances, so they get their own query that is only
    /// collected while a GPU / top-apps tile is on screen.
    gpu_query: PDH_HQUERY,
    gpu_eng: Option<PDH_HCOUNTER>,
    gpu_ded: Option<PDH_HCOUNTER>,
    gpu_shared: Option<PDH_HCOUNTER>,
    disk: Option<PDH_HCOUNTER>,
    cpu_perf: Option<PDH_HCOUNTER>,
    cpu_base: Option<PDH_HCOUNTER>,
    /// "% Processor Time": the share of time the processors were busy. Deliberately not Task Manager's
    /// "% Processor Utility", which multiplies that by the clock / base clock ratio: in turbo (1.35x on a
    /// 5.25 GHz Ryzen with a 3.9 GHz base) 10 busy threads of 12 read 120 % there and are shown as 100 %.
    cpu_busy: Option<PDH_HCOUNTER>,
    core_busy: Option<PDH_HCOUNTER>,
    /// Per logical processor "% Processor Performance": its clock as a share of the base clock.
    core_perf: Option<PDH_HCOUNTER>,
    thermal: Option<PDH_HCOUNTER>,
    energy: Option<PDH_HCOUNTER>,
}

// PDH handles are plain integers; we only ever touch them from the sampler thread.
unsafe impl Send for Pdh {}

#[derive(Default, Clone, Copy)]
pub struct GpuCounters {
    pub usage: f32,
    pub dedicated_used: u64,
    pub shared_used: u64,
}

impl Pdh {
    pub fn new() -> Option<Self> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut gpu_query = PDH_HQUERY::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut gpu_query) != 0 {
                return None;
            }
            let add_to = |q: PDH_HQUERY, path: PCWSTR| -> Option<PDH_HCOUNTER> {
                let mut c = PDH_HCOUNTER::default();
                (PdhAddEnglishCounterW(q, path, 0, &mut c) == 0).then_some(c)
            };
            let add = |path: PCWSTR| add_to(query, path);
            let p = Self {
                query,
                gpu_query,
                gpu_eng: add_to(gpu_query, w!("\\GPU Engine(*)\\Utilization Percentage")),
                gpu_ded: add_to(gpu_query, w!("\\GPU Adapter Memory(*)\\Dedicated Usage")),
                gpu_shared: add_to(gpu_query, w!("\\GPU Adapter Memory(*)\\Shared Usage")),
                // "% Disk Time" is queue-length based and goes far above 100 on SSD / NVMe; Task Manager's
                // "Active time" is 100 minus "% Idle Time".
                disk: add(w!("\\PhysicalDisk(_Total)\\% Idle Time")),
                cpu_perf: add(w!("\\Processor Information(_Total)\\% Processor Performance")),
                cpu_base: add(w!("\\Processor Information(_Total)\\Processor Frequency")),
                cpu_busy: add(w!("\\Processor Information(_Total)\\% Processor Time")),
                core_busy: add(w!("\\Processor Information(*)\\% Processor Time")),
                core_perf: add(w!("\\Processor Information(*)\\% Processor Performance")),
                thermal: add(w!("\\Thermal Zone Information(*)\\High Precision Temperature")),
                energy: add(w!("\\Energy Meter(*)\\Power")),
            };
            // Rate counters need a first sample to diff against.
            PdhCollectQueryData(query);
            PdhCollectQueryData(gpu_query);
            Some(p)
        }
    }

    pub fn collect(&self) {
        unsafe {
            PdhCollectQueryData(self.query);
        }
    }

    pub fn collect_gpu(&self) {
        unsafe {
            PdhCollectQueryData(self.gpu_query);
        }
    }

    fn value(&self, c: Option<PDH_HCOUNTER>) -> Option<f64> {
        unsafe {
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(c?, PDH_FMT_DOUBLE, None, &mut v) != 0 || v.CStatus > 1 {
                return None;
            }
            Some(v.Anonymous.doubleValue)
        }
    }

    /// Current CPU clock in MHz (base clock scaled by "% Processor Performance", like Task Manager).
    pub fn cpu_mhz(&self) -> Option<u64> {
        let (perf, base) = (self.value(self.cpu_perf)?, self.value(self.cpu_base)?);
        (base > 0.0 && perf > 0.0).then(|| (base * perf / 100.0).round() as u64)
    }

    /// Whole-CPU load: the share of time the processors were busy (0..100).
    pub fn cpu_busy(&self) -> Option<f32> {
        self.value(self.cpu_busy).map(|v| v.clamp(0.0, 100.0) as f32)
    }

    /// Per logical processor, in processor order ("group,index" instances; the totals are left out).
    pub fn core_busy(&self) -> Vec<f32> {
        let Some(c) = self.core_busy else { return Vec::new() };
        core_order(unsafe { read_array(c) })
    }

    /// Current clock of every logical processor in MHz (processor order), like `cpu_mhz` for the whole CPU.
    pub fn core_mhz(&self) -> Vec<u32> {
        let (Some(c), Some(base)) = (self.core_perf, self.value(self.cpu_base)) else { return Vec::new() };
        if base <= 0.0 {
            return Vec::new();
        }
        // core_order clamps to 0..100 (it is made for loads) and a clock share goes above 100 in turbo: scale it down first.
        let rows: Vec<(String, f64)> = unsafe { read_array(c) }.into_iter().map(|(n, v)| (n, v / 10.0)).collect();
        core_order(rows).into_iter().map(|share| (share as f64 * 10.0 * base / 100.0).round() as u32).collect()
    }

    /// Hottest ACPI thermal zone in °C. On most laptops/desktops this is the CPU package, but ACPI
    /// zones are board-defined, so it is an approximation (no driver / admin rights needed).
    pub fn cpu_temp_c(&self) -> Option<f32> {
        let rows = unsafe { read_array(self.thermal?) };
        rows.into_iter()
            // High-precision value is in tenths of a Kelvin.
            .map(|(_, v)| (v / 10.0 - 273.15) as f32)
            .filter(|t| (10.0..130.0).contains(t))
            .reduce(f32::max)
    }

    /// CPU package power in watts from Windows' "Energy Meter" counters (Intel RAPL). Needs no admin
    /// rights, but only exists where the platform exposes an energy meter: Intel machines yes, AMD
    /// Ryzen machines report no instances, and then this is None.
    pub fn cpu_power_w(&self) -> Option<f32> {
        let rows = unsafe { read_array(self.energy?) };
        let pick = |key: &str| {
            rows.iter()
                .find(|(n, _)| n.to_lowercase().contains(key))
                // Reported in milliwatts.
                .map(|(_, v)| (*v / 1000.0) as f32)
        };
        pick("pkg").or_else(|| pick("pp0")).filter(|w| (0.05..500.0).contains(w))
    }

    /// Disk active time, 0..100 (same definition as Task Manager).
    pub fn disk_active(&self) -> f32 {
        let Some(c) = self.disk else { return 0.0 };
        unsafe {
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(c, PDH_FMT_DOUBLE, None, &mut v) != 0 || v.CStatus > 1 {
                return 0.0;
            }
            (100.0 - v.Anonymous.doubleValue as f32).clamp(0.0, 100.0)
        }
    }

    /// Per-adapter GPU counters keyed by LUID string, e.g. `luid_0x00000000_0x000145a3`.
    /// Also returns the load per process id (its busiest engine), for the "top apps" list. Work Windows cannot
    /// tie to a process (a WSL 2 / Hyper-V guest using the card) is under pid 0.
    pub fn gpus(&self) -> (HashMap<String, GpuCounters>, HashMap<u32, f32>) {
        let mut out: HashMap<String, GpuCounters> = HashMap::new();
        let mut pids: HashMap<u32, f32> = HashMap::new();

        if let Some(c) = self.gpu_eng {
            let (adapters, by_pid) = engine_loads(&unsafe { read_array(c) });
            pids = by_pid;
            for (luid, load) in adapters {
                out.entry(luid).or_default().usage = load;
            }
        }
        if let Some(c) = self.gpu_ded {
            for (name, v) in unsafe { read_array(c) } {
                if let Some(luid) = luid_of(&name.to_lowercase()) {
                    out.entry(luid).or_default().dedicated_used = v as u64;
                }
            }
        }
        if let Some(c) = self.gpu_shared {
            for (name, v) in unsafe { read_array(c) } {
                if let Some(luid) = luid_of(&name.to_lowercase()) {
                    out.entry(luid).or_default().shared_used = v as u64;
                }
            }
        }
        for g in out.values_mut() {
            g.usage = g.usage.clamp(0.0, 100.0);
        }
        (out, pids)
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
            PdhCloseQuery(self.gpu_query);
        }
    }
}

/// `pid_1_luid_0x00000000_0x000145a3_phys_0_eng_0_...` or `luid_0x..._0x..._phys_0` -> `luid_0x00000000_0x000145a3`
fn luid_of(name: &str) -> Option<String> {
    let start = name.find("luid_")?;
    let rest = &name[start..];
    let end = rest.find("_phys").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// `pid_10028_luid_...` -> 10028
fn pid_of(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

/// One GPU Engine instance, `pid_10028_luid_0x00000000_0x000145a3_phys_0_eng_3_engtype_copy` (lower case):
/// the process, the adapter, and which engine of the adapter ("phys_0_eng_3_engtype_copy").
#[derive(Debug, PartialEq)]
struct EngineName {
    pid: Option<u32>,
    luid: String,
    engine: String,
}

fn parse_engine(name: &str) -> Option<EngineName> {
    let luid = luid_of(name)?;
    let rest = &name[name.find("luid_")?..];
    let engine = rest[rest.find("_phys")?..].trim_start_matches('_');
    engine.contains("engtype_").then(|| EngineName { pid: pid_of(name), luid, engine: engine.to_string() })
}

/// Loads from the `GPU Engine` counter rows (name, %): per adapter and per process id, both 0..100.
///
/// Every engine of a card is its own unit that can be 100 % busy: a card has one 3D engine but several copy
/// engines, and adding those up (what summing per engine *type* did) read 100 % when six copy engines were
/// each 20 % busy. So: the load of an engine is what all processes together put on it (several contexts of one
/// process, or of several, share the same instance name and add up), an adapter's load is its busiest engine
/// (what Task Manager shows), and a process's load is the busiest engine it uses on any adapter.
fn engine_loads(rows: &[(String, f64)]) -> (HashMap<String, f32>, HashMap<u32, f32>) {
    let mut per_engine: HashMap<(String, String), f64> = HashMap::new();
    let mut per_proc_engine: HashMap<(u32, String, String), f64> = HashMap::new();
    for (name, v) in rows {
        let Some(e) = parse_engine(&name.to_lowercase()) else { continue };
        // A counter that failed (NaN) or went negative is no load.
        let v = if v.is_finite() { v.max(0.0) } else { 0.0 };
        if let Some(pid) = e.pid {
            *per_proc_engine.entry((pid, e.luid.clone(), e.engine.clone())).or_default() += v;
        }
        *per_engine.entry((e.luid, e.engine)).or_default() += v;
    }
    let mut adapters: HashMap<String, f32> = HashMap::new();
    for ((luid, _), v) in per_engine {
        let a = adapters.entry(luid).or_default();
        *a = a.max(v.min(100.0) as f32);
    }
    let mut pids: HashMap<u32, f32> = HashMap::new();
    for ((pid, _, _), v) in per_proc_engine {
        let p = pids.entry(pid).or_default();
        *p = p.max(v.min(100.0) as f32);
    }
    (adapters, pids)
}

/// "0,3" -> processor 3 of group 0; "_Total" / "0,_Total" are dropped. Sorted by group, then index.
fn core_order(rows: Vec<(String, f64)>) -> Vec<f32> {
    let mut cores: Vec<((u32, u32), f32)> = rows
        .into_iter()
        .filter_map(|(name, v)| {
            let (g, i) = name.split_once(',')?;
            Some(((g.trim().parse().ok()?, i.trim().parse().ok()?), v.clamp(0.0, 100.0) as f32))
        })
        .collect();
    cores.sort_by_key(|(k, _)| *k);
    cores.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "reads this machine's counters: run with --ignored --nocapture"]
    fn print_busy_time_vs_sysinfo() {
        let p = super::Pdh::new().unwrap();
        let mut sys = sysinfo::System::new();
        sys.refresh_cpu_usage();
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            p.collect();
            sys.refresh_cpu_usage();
            println!("busy {:?}  sysinfo {:.1}  per core {:?}", p.cpu_busy(), sys.global_cpu_usage(), p.core_busy());
        }
    }

    fn row(name: &str, v: f64) -> (String, f64) {
        (name.to_string(), v)
    }

    const DGPU: &str = "luid_0x00000000_0x0fe650bb";
    const IGPU: &str = "luid_0x00000000_0x00014fbf";

    #[test]
    fn engine_names_are_taken_apart() {
        let e = super::parse_engine("pid_10028_luid_0x00000000_0x0fe650bb_phys_0_eng_3_engtype_copy").unwrap();
        assert_eq!(e.pid, Some(10028));
        assert_eq!(e.luid, DGPU);
        assert_eq!(e.engine, "phys_0_eng_3_engtype_copy");
        assert!(super::parse_engine("pid_0_luid_0x00000000_0x0fe650bb_phys_0_eng_0_engtype_3d").is_some());
        // Not engine instances (or too damaged to place): left out, never a panic.
        for n in ["", "_total", "pid_5_luid_0x1_0x2", "luid_0x1_0x2_phys_0"] {
            assert!(super::parse_engine(n).is_none(), "{n}");
        }
        // No process in the name: still an engine of the adapter.
        assert_eq!(super::parse_engine("luid_0x1_0x2_phys_0_eng_0_engtype_3d").unwrap().pid, None);
    }

    #[test]
    fn several_copy_engines_do_not_add_up_to_a_full_card() {
        let mut rows = vec![row(&format!("pid_10_{DGPU}_phys_0_eng_0_engtype_3D"), 12.0)];
        for eng in [3, 4, 9, 10, 11, 12] {
            rows.push(row(&format!("pid_10_{DGPU}_phys_0_eng_{eng}_engtype_Copy"), 20.0));
        }
        let (adapters, pids) = super::engine_loads(&rows);
        assert_eq!(adapters[DGPU], 20.0, "the busiest engine, not 6 x 20 = 120");
        assert_eq!(pids[&10], 20.0);
    }

    #[test]
    fn processes_sharing_an_engine_add_up_and_the_busiest_engine_counts_for_each() {
        let rows = vec![
            row(&format!("pid_100_{DGPU}_phys_0_eng_0_engtype_3D"), 30.0),
            row(&format!("pid_200_{DGPU}_phys_0_eng_0_engtype_3D"), 25.0),
            row(&format!("pid_200_{DGPU}_phys_0_eng_6_engtype_VideoEncode"), 40.0),
            // two contexts of one process on one engine come as two instances with the same name
            row(&format!("pid_100_{DGPU}_phys_0_eng_0_engtype_3D"), 5.0),
        ];
        let (adapters, pids) = super::engine_loads(&rows);
        assert_eq!(adapters[DGPU], 60.0, "3D: 30 + 25 + 5");
        assert_eq!(pids[&100], 35.0);
        assert_eq!(pids[&200], 40.0, "its video encoder is busier than its share of 3D");
    }

    #[test]
    fn a_process_is_not_charged_twice_for_using_two_cards() {
        let rows = vec![
            row(&format!("pid_7_{DGPU}_phys_0_eng_0_engtype_3D"), 30.0),
            row(&format!("pid_7_{IGPU}_phys_0_eng_0_engtype_3D"), 20.0),
        ];
        let (adapters, pids) = super::engine_loads(&rows);
        assert_eq!((adapters[DGPU], adapters[IGPU]), (30.0, 20.0));
        assert_eq!(pids[&7], 30.0, "not 50 %");
    }

    #[test]
    fn work_without_a_process_is_kept_under_pid_zero() {
        // What a WSL 2 guest running CUDA looks like on a real PC: two contexts, no process, mixed-case names.
        let rows = vec![
            row("pid_0_luid_0x00000000_0x0FE650BB_phys_0_eng_0_engtype_3D", 50.76),
            row("pid_0_luid_0x00000000_0x0FE650BB_phys_0_eng_0_engtype_3D", 26.58),
            row(&format!("pid_15144_{DGPU}_phys_0_eng_0_engtype_3D"), 0.06),
        ];
        let (adapters, pids) = super::engine_loads(&rows);
        assert!((adapters[DGPU] - 77.4).abs() < 0.01, "the card is {} % busy", adapters[DGPU]);
        assert!((pids[&0] - 77.34).abs() < 0.01);
        assert!((pids[&15144] - 0.06).abs() < 0.001);
    }

    #[test]
    fn bad_values_and_overload_stay_inside_0_to_100() {
        let rows = vec![
            row(&format!("pid_1_{DGPU}_phys_0_eng_0_engtype_3D"), f64::NAN),
            row(&format!("pid_2_{DGPU}_phys_0_eng_0_engtype_3D"), -5.0),
            row(&format!("pid_3_{DGPU}_phys_0_eng_0_engtype_3D"), 80.0),
            row(&format!("pid_4_{DGPU}_phys_0_eng_0_engtype_3D"), 70.0),
            row("garbage", 90.0),
        ];
        let (adapters, pids) = super::engine_loads(&rows);
        assert_eq!(adapters[DGPU], 100.0);
        assert_eq!((pids[&1], pids[&2], pids[&3], pids[&4]), (0.0, 0.0, 80.0, 70.0));
        assert!(super::engine_loads(&[]).0.is_empty());
    }

    #[test]
    fn cores_come_in_processor_order_without_totals() {
        let rows = vec![
            ("0,10".to_string(), 10.0),
            ("_Total".to_string(), 50.0),
            ("0,2".to_string(), 2.0),
            ("1,0".to_string(), 100.0),
            ("0,_Total".to_string(), 40.0),
            ("0,0".to_string(), 120.0),
        ];
        assert_eq!(super::core_order(rows), vec![100.0, 2.0, 10.0, 100.0]);
    }
}

pub(crate) unsafe fn read_array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
    let (mut size, mut count) = (0u32, 0u32);
    let st = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
    if st != PDH_MORE_DATA || size == 0 {
        return Vec::new();
    }
    // u64 backing store keeps the item array 8-byte aligned.
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let ptr = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(ptr)) != 0 {
        return Vec::new();
    }
    std::slice::from_raw_parts(ptr, count as usize)
        .iter()
        .map(|item| {
            let name = item.szName.to_string().unwrap_or_default();
            let v = if item.FmtValue.CStatus <= 1 { item.FmtValue.Anonymous.doubleValue } else { 0.0 };
            (name, v)
        })
        .collect()
}

