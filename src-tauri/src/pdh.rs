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
    /// Also returns the load per process id (busiest engine type), for the "top apps" list.
    pub fn gpus(&self) -> (HashMap<String, GpuCounters>, HashMap<u32, f32>) {
        let mut out: HashMap<String, GpuCounters> = HashMap::new();
        let mut pids: HashMap<u32, f32> = HashMap::new();

        if let Some(c) = self.gpu_eng {
            // Sum engine load per (adapter, engine type); the adapter load is the busiest engine type,
            // which is how Task Manager reports it.
            let mut per_type: HashMap<(String, String), f64> = HashMap::new();
            let mut per_pid: HashMap<(u32, String), f64> = HashMap::new();
            for (name, v) in unsafe { read_array(c) } {
                let name = name.to_lowercase();
                let (Some(luid), Some(eng)) = (luid_of(&name), name.split("engtype_").nth(1)) else {
                    continue;
                };
                if let Some(pid) = pid_of(&name) {
                    *per_pid.entry((pid, eng.to_string())).or_default() += v;
                }
                *per_type.entry((luid, eng.to_string())).or_default() += v;
            }
            for ((pid, _), v) in per_pid {
                let e = pids.entry(pid).or_default();
                *e = e.max(v as f32);
            }
            for ((luid, _), v) in per_type {
                let e = out.entry(luid).or_default();
                e.usage = e.usage.max(v as f32);
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

