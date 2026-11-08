//! Chart history, kept by the backend so the charts also cover the time the popup was closed.
//!
//! Every sample (1 s apart while the popup is open, 2 s while it sits in the tray) is recorded here, for the
//! longest chart window the user can pick (1 hour). The UI asks for its window when it starts, then only for
//! what it missed while the popup was closed, and keeps appending the samples it receives live. The keys are
//! the UI's chart keys ("cpu.usage", "gpu.<luid>.temp", ...).

use crate::metrics::{Metrics, ProcInfo};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

/// How far back the longest chart window reaches (the "1 hour" choice in Settings).
pub const HISTORY_SECS: u64 = 3600;
/// The battery charge chart moves slowly: one point every 5 s for 10 minutes.
const CHARGE_EVERY_MS: u64 = 5_000;
const CHARGE_SECS: u64 = 600;
const GIB: u64 = 1024 * 1024 * 1024;
/// A timestamp this far behind the newest one means the system clock was set back.
const CLOCK_JUMP_MS: u64 = 5_000;
/// Apps kept per list for "who caused this peak?".
const TOPS_KEPT: usize = 3;
/// A process measurement covers the time since the previous one; after a longer pause (sleep) it covers this much.
const TOPS_MAX_SPAN_MS: u64 = 10_000;
/// The newest chart samples come after the last process measurement: it stands in for them this long.
const TOPS_AHEAD_MS: u64 = 7_000;

/// Who used the most between two process measurements (`from`..`t`): CPU %, memory bytes, disk bytes/s, GPU %.
struct TopsAt {
    t: u64,
    from: u64,
    cpu: Vec<(Arc<str>, f64)>,
    mem: Vec<(Arc<str>, f64)>,
    disk: Vec<(Arc<str>, f64)>,
    /// None: the GPU counters weren't running then.
    gpu: Option<Vec<(Arc<str>, f64)>>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct TopApp {
    pub name: String,
    pub value: f64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TopAppsDto {
    /// The measurement window the apps were measured over (ms).
    pub from: u64,
    pub to: u64,
    pub apps: Vec<TopApp>,
}

#[derive(Clone)]
struct GpuPoint {
    /// Index into `History::luids` (an hour of samples would otherwise hold thousands of copies of the name).
    luid: u8,
    usage: f32,
    temp: Option<f32>,
    vram: Option<f32>,
    power: Option<f32>,
}

/// One physical disk in one sample (the UI's "disk.<n>.active" / ".read" / ".write").
#[derive(Clone)]
struct DiskPoint {
    index: u32,
    active: f32,
    read: f64,
    write: f64,
}

#[derive(Clone)]
struct Point {
    t: u64,
    cpu: f32,
    cpu_temp: Option<f32>,
    cpu_power: Option<f32>,
    cpu_clock: Option<f32>,
    ram: f32,
    swap: Option<f32>,
    down: f64,
    up: f64,
    disk_active: f32,
    disk_read: f64,
    disk_write: f64,
    phys: Vec<DiskPoint>,
    bat_power: Option<f32>,
    gpus: Vec<GpuPoint>,
}

fn share(used: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (used as f64 / total as f64 * 100.0).min(100.0) as f32)
}

fn intern(luids: &mut Vec<String>, luid: &str) -> u8 {
    match luids.iter().position(|l| l == luid) {
        Some(i) => i as u8,
        None => {
            luids.push(luid.to_string());
            (luids.len() - 1).min(u8::MAX as usize) as u8
        }
    }
}

/// Two decimals are all a chart can show; f32 values would otherwise reach the UI as 84.4000015258789.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

impl Point {
    fn of(m: &Metrics, luids: &mut Vec<String>) -> Self {
        Point {
            t: m.ts_ms,
            cpu: m.cpu_usage,
            cpu_temp: m.cpu_temp_c,
            cpu_power: m.cpu_power_w,
            cpu_clock: (m.cpu_freq_mhz > 0).then_some(m.cpu_freq_mhz as f32),
            ram: share(m.mem_used, m.mem_total).unwrap_or(0.0),
            swap: share(m.swap_used, m.swap_total),
            down: m.net_down,
            up: m.net_up,
            disk_active: m.disk_active,
            disk_read: m.disk_read,
            disk_write: m.disk_write,
            phys: m
                .phys_disks
                .iter()
                .map(|d| DiskPoint { index: d.index, active: d.active, read: d.read, write: d.write })
                .collect(),
            bat_power: m
                .battery
                .detail
                .as_ref()
                .filter(|_| m.battery.present)
                .map(|d| (d.rate_mw.unsigned_abs() as f64 / 1000.0) as f32),
            gpus: m
                .gpus
                .iter()
                .map(|g| GpuPoint {
                    luid: intern(luids, &g.luid),
                    usage: g.usage,
                    temp: g.sensors.temp_c,
                    // Integrated GPUs work from shared system memory (same rule as the tile).
                    vram: if g.vram_total >= GIB { share(g.vram_used, g.vram_total) } else { share(g.shared_used, g.shared_total) },
                    power: g.sensors.power_w,
                })
                .collect(),
        }
    }
}

#[derive(Default)]
pub struct History {
    points: VecDeque<Point>,
    charge: VecDeque<(u64, u8)>,
    luids: Vec<String>,
    tops: VecDeque<TopsAt>,
    /// One copy of each app name (an hour of measurements names the same few apps thousands of times).
    names: HashMap<String, Arc<str>>,
}

#[derive(Serialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDto {
    /// true: this is the whole window (replace what you have); false: only the samples after `since`.
    pub full: bool,
    pub times: Vec<u64>,
    /// Every series is as long as `times` (null = not measured then).
    pub series: BTreeMap<String, Vec<Option<f64>>>,
    pub charge_times: Vec<u64>,
    pub charge: Vec<u8>,
}

impl History {
    pub fn push(&mut self, m: &Metrics) {
        if m.cpu_cores == 0 {
            return; // not a real sample (the empty start value)
        }
        if let Some(last) = self.points.back().map(|p| p.t) {
            if m.ts_ms <= last {
                if last - m.ts_ms < CLOCK_JUMP_MS {
                    return; // the same sample again
                }
                // The clock was set back (time sync): start over instead of freezing until it catches up.
                self.points.clear();
                self.charge.clear();
                self.tops.clear();
            }
        }
        self.points.push_back(Point::of(m, &mut self.luids));
        let oldest = m.ts_ms.saturating_sub(HISTORY_SECS * 1000);
        while self.points.front().is_some_and(|p| p.t < oldest) {
            self.points.pop_front();
        }
        if m.procs_sampled {
            self.push_tops(m);
        }
        while self.tops.front().is_some_and(|r| r.t < oldest) {
            self.tops.pop_front();
        }
        if m.battery.present && self.charge.back().is_none_or(|&(t, _)| m.ts_ms.saturating_sub(t) >= CHARGE_EVERY_MS) {
            self.charge.push_back((m.ts_ms, m.battery.percent));
            let oldest = m.ts_ms.saturating_sub(CHARGE_SECS * 1000);
            while self.charge.front().is_some_and(|&(t, _)| t < oldest) {
                self.charge.pop_front();
            }
        }
    }

    fn push_tops(&mut self, m: &Metrics) {
        if self.names.len() > 4096 {
            self.names.clear(); // only the sharing of names is lost
        }
        let names = &mut self.names;
        let mut keep = |list: &[ProcInfo], value: fn(&ProcInfo) -> f64, least: f64| -> Vec<(Arc<str>, f64)> {
            list.iter()
                .map(|p| (p, value(p)))
                .filter(|(_, v)| *v >= least)
                .take(TOPS_KEPT)
                .map(|(p, v)| {
                    let name = names.entry(p.name.clone()).or_insert_with(|| Arc::from(p.name.as_str())).clone();
                    (name, v)
                })
                .collect()
        };
        // Below 0.05 % reads "0.0%": not worth naming.
        let cpu = keep(&m.top_cpu, |p| p.cpu as f64, 0.05);
        let mem = keep(&m.top_mem, |p| p.mem as f64, 1.0);
        let disk = keep(&m.top_disk, |p| p.disk, 1.0);
        let gpu = m.procs_gpu_fresh.then(|| keep(&m.top_gpu, |p| p.gpu as f64, 0.05));
        let prev = self.tops.back().map(|r| r.t).filter(|&t| t < m.ts_ms && m.ts_ms - t <= TOPS_MAX_SPAN_MS);
        let from = prev.unwrap_or(m.ts_ms.saturating_sub(TOPS_MAX_SPAN_MS / 2));
        self.tops.push_back(TopsAt { t: m.ts_ms, from, cpu, mem, disk, gpu });
    }

    /// Who used the most CPU / memory / disk / GPU ("cpu", "mem", "disk", "gpu") at chart time `t`: the process
    /// measurement whose window holds `t` (the newest one also stands in for the few samples after it).
    pub fn tops_at(&self, t: u64, kind: &str) -> Option<TopAppsDto> {
        let i = self.tops.partition_point(|r| r.t < t);
        let rec = match self.tops.get(i) {
            Some(r) if r.from <= t => r,
            Some(_) => return None, // a pause (sleep) with no measurement
            None => self.tops.back().filter(|r| t - r.t <= TOPS_AHEAD_MS)?,
        };
        let list = match kind {
            "cpu" => &rec.cpu,
            "mem" => &rec.mem,
            "disk" => &rec.disk,
            "gpu" => rec.gpu.as_ref()?,
            _ => return None,
        };
        Some(TopAppsDto {
            from: rec.from,
            to: rec.t,
            apps: list.iter().map(|(n, v)| TopApp { name: n.to_string(), value: *v }).collect(),
        })
    }

    /// The last `secs` seconds. With `since` (the newest sample the caller already has) only what came after it,
    /// as long as that sample is still here and inside the window; otherwise (first call, the popup was closed for
    /// longer than the window, the clock was set back) the whole window, marked `full`.
    pub fn snapshot(&self, since: Option<u64>, secs: u64) -> HistoryDto {
        let newest = self.points.back().map_or(0, |p| p.t);
        let oldest = newest.saturating_sub(secs.min(HISTORY_SECS) * 1000);
        let known = since.filter(|&s| {
            let i = self.points.partition_point(|p| p.t < s);
            s >= oldest && i < self.points.len() && self.points[i].t == s
        });
        let start = match known {
            Some(s) => self.points.partition_point(|p| p.t <= s),
            None => self.points.partition_point(|p| p.t < oldest),
        };
        let pts = || self.points.range(start..);
        let mut series: BTreeMap<String, Vec<Option<f64>>> = BTreeMap::new();
        let mut col = |key: &str, f: &dyn Fn(&Point) -> Option<f64>| {
            series.insert(key.into(), pts().map(f).collect());
        };
        // Percentages, temperatures and watts to 2 decimals; clocks and byte rates whole.
        col("cpu.usage", &|p| Some(r2(p.cpu as f64)));
        col("cpu.temp", &|p| p.cpu_temp.map(|v| r2(v as f64)));
        col("cpu.power", &|p| p.cpu_power.map(|v| r2(v as f64)));
        col("cpu.clock", &|p| p.cpu_clock.map(|v| v.round() as f64));
        col("mem.ram", &|p| Some(r2(p.ram as f64)));
        col("mem.swap", &|p| p.swap.map(|v| r2(v as f64)));
        col("net.down", &|p| Some(p.down.round()));
        col("net.up", &|p| Some(p.up.round()));
        col("disk.active", &|p| Some(r2(p.disk_active as f64)));
        col("disk.read", &|p| Some(p.disk_read.round()));
        col("disk.write", &|p| Some(p.disk_write.round()));
        col("bat.power", &|p| p.bat_power.map(|v| r2(v as f64)));
        // Every physical disk that was measured in the window, under its disk number.
        let mut disks: Vec<u32> = Vec::new();
        for p in pts() {
            for d in &p.phys {
                if !disks.contains(&d.index) {
                    disks.push(d.index);
                }
            }
        }
        disks.sort_unstable();
        for idx in disks {
            let one = |p: &Point| p.phys.iter().find(|d| d.index == idx).cloned();
            let mut put = |metric: &str, f: &dyn Fn(&DiskPoint) -> f64| {
                series.insert(format!("disk.{idx}.{metric}"), pts().map(|p| one(p).map(|d| f(&d))).collect());
            };
            put("active", &|d| r2(d.active as f64));
            put("read", &|d| d.read.round());
            put("write", &|d| d.write.round());
        }
        let mut seen: Vec<u8> = Vec::new();
        for p in pts() {
            for g in &p.gpus {
                if !seen.contains(&g.luid) {
                    seen.push(g.luid);
                }
            }
        }
        for idx in seen {
            let luid = &self.luids[idx as usize];
            let gpu = |p: &Point| p.gpus.iter().find(|g| g.luid == idx).cloned();
            let mut put = |metric: &str, f: &dyn Fn(&GpuPoint) -> Option<f64>| {
                series.insert(format!("gpu.{luid}.{metric}"), pts().map(|p| gpu(p).and_then(|g| f(&g))).collect());
            };
            put("usage", &|g| Some(r2(g.usage as f64)));
            put("temp", &|g| g.temp.map(|v| r2(v as f64)));
            put("vram", &|g| g.vram.map(|v| r2(v as f64)));
            put("power", &|g| g.power.map(|v| r2(v as f64)));
        }
        HistoryDto {
            full: known.is_none(),
            times: pts().map(|p| p.t).collect(),
            series,
            charge_times: self.charge.iter().map(|c| c.0).collect(),
            charge: self.charge.iter().map(|c| c.1).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::GpuNow;

    fn metrics(t: u64, cpu: f32, gpu: bool) -> Metrics {
        let mut m = Metrics { ts_ms: t, cpu_cores: 8, cpu_usage: cpu, mem_total: 100, mem_used: 25, ..Default::default() };
        if gpu {
            m.gpus.push(GpuNow { luid: "L1".into(), usage: 50.0, vram_total: 8 * GIB, vram_used: 2 * GIB, ..Default::default() });
        }
        m
    }

    fn app(name: &str, cpu: f32, mem: u64) -> ProcInfo {
        ProcInfo { name: name.into(), cpu, mem, ..Default::default() }
    }

    fn disk(index: u32, letters: &str, active: f32, read: f64, write: f64) -> crate::pdh::PhysDisk {
        crate::pdh::PhysDisk { index, letters: letters.into(), active, read, write }
    }

    #[test]
    fn every_physical_disk_has_its_own_series_under_its_disk_number() {
        let mut h = History::default();
        for i in 0..5u64 {
            let mut m = metrics(1_000_000 + i * 1000, 1.0, false);
            m.phys_disks = vec![disk(0, "C:", 10.0 + i as f32, 1.0e6, 2.0e3), disk(2, "E:", 0.4, 0.0, 9.5e6)];
            h.push(&m);
        }
        let s = h.snapshot(None, 180).series;
        assert_eq!(s["disk.0.active"], [Some(10.0), Some(11.0), Some(12.0), Some(13.0), Some(14.0)]);
        assert_eq!(s["disk.0.read"], [Some(1.0e6); 5]);
        assert_eq!(s["disk.0.write"], [Some(2.0e3); 5]);
        assert_eq!(s["disk.2.active"], [Some(0.4); 5]);
        assert_eq!(s["disk.2.write"], [Some(9.5e6); 5]);
        assert!(!s.contains_key("disk.1.active"), "a disk that was never there has no series");
        assert_eq!(s["disk.active"].len(), 5, "the all-disks series stay as they were");
    }

    #[test]
    fn a_disk_that_appears_later_is_empty_before_and_series_stay_as_long_as_the_times() {
        let mut h = History::default();
        for i in 0..6u64 {
            let mut m = metrics(1_000_000 + i * 1000, 1.0, false);
            if i >= 3 {
                m.phys_disks = vec![disk(1, "D:", 50.0, 3.0e6, 0.0)];
            }
            h.push(&m);
        }
        let d = h.snapshot(None, 180);
        assert_eq!(d.series["disk.1.active"], [None, None, None, Some(50.0), Some(50.0), Some(50.0)]);
        assert!(d.series.values().all(|v| v.len() == d.times.len()));
        // only the newest samples: the series is as long as that
        let d = h.snapshot(Some(1_000_000 + 3000), 180);
        assert_eq!(d.series["disk.1.read"], [Some(3.0e6), Some(3.0e6)]);
    }

    #[test]
    fn without_per_disk_counters_there_are_no_disk_series_but_the_totals() {
        let mut h = History::default();
        h.push(&metrics(1_000_000, 1.0, false));
        let s = h.snapshot(None, 180).series;
        assert!(s.contains_key("disk.active") && !s.keys().any(|k| k.starts_with("disk.0.")));
    }

    #[test]
    fn the_apps_behind_a_chart_point_are_those_of_the_measurement_covering_it() {
        let mut h = History::default();
        let t0 = 1_000_000;
        for i in 0..20u64 {
            let mut m = metrics(t0 + i * 1000, 10.0, false);
            // a process measurement every 3rd sample, like the sampler on screen
            if i % 3 == 0 {
                m.procs_sampled = true;
                let (a, b) = if i < 9 { ("chrome.exe", "Code.exe") } else { ("vmmem", "chrome.exe") };
                m.top_cpu = vec![app(a, 40.0 + i as f32, 0), app(b, 5.0, 0), app("idle", 0.01, 0), app("x", 1.0, 0)];
                m.top_mem = vec![app("vmmem", 0.0, 8 << 30)];
            }
            h.push(&m);
        }
        let names = |t: u64| h.tops_at(t, "cpu").map(|d| d.apps.iter().map(|a| a.name.clone()).collect::<Vec<_>>());
        // t0+4s lies between the measurements at t0+3s and t0+6s: the t0+6s one covers it
        let d = h.tops_at(t0 + 4000, "cpu").unwrap();
        assert_eq!((d.from, d.to), (t0 + 3000, t0 + 6000));
        assert_eq!(d.apps[0], TopApp { name: "chrome.exe".into(), value: 46.0 });
        assert_eq!(names(t0 + 10_000).unwrap(), ["vmmem", "chrome.exe", "x"], "near-zero apps are left out, 3 at most");
        assert_eq!(names(t0 + 19_000).unwrap()[0], "vmmem", "the newest samples use the last measurement");
        assert_eq!(names(t0 + 30_000), None, "too far past the last measurement");
        assert_eq!(h.tops_at(t0 + 4000, "mem").unwrap().apps[0].value, (8u64 << 30) as f64);
        assert!(h.tops_at(t0 + 4000, "gpu").is_none(), "no GPU counters were running");
        assert!(h.tops_at(t0 + 4000, "net").is_none());
    }

    #[test]
    fn keeps_an_hour_and_hands_out_the_window_asked_for() {
        let mut h = History::default();
        for i in 0..4000u64 {
            h.push(&metrics(1_000_000 + i * 1000, (i % 100) as f32, false));
        }
        assert_eq!(h.snapshot(None, 3600).times.len(), 3601, "one hour, both ends included");
        assert_eq!(h.snapshot(None, 900).times.len(), 901);
        assert_eq!(h.snapshot(None, 99_999).times.len(), 3601, "never more than is kept");
        assert!(h.snapshot(None, 180).full);
    }

    #[test]
    fn only_what_the_ui_missed_is_sent_again() {
        let mut h = History::default();
        for i in 0..100u64 {
            h.push(&metrics(1_000_000 + i * 1000, 1.0, true));
        }
        let d = h.snapshot(Some(1_000_000 + 89_000), 180);
        assert!(!d.full);
        assert_eq!(d.times, (90..100u64).map(|i| 1_000_000 + i * 1000).collect::<Vec<_>>());
        assert_eq!(d.series["cpu.usage"].len(), 10);
        assert_eq!(d.series["gpu.L1.usage"].len(), 10);
        let d = h.snapshot(Some(1_000_000 + 99_000), 180);
        assert!(!d.full && d.times.is_empty(), "up to date: nothing to send");
        // A sample the backend never had, one from before the window, or none at all: the whole window again.
        assert!(h.snapshot(Some(1_000_000 + 89_500), 180).full);
        assert!(h.snapshot(Some(5_000), 180).full);
        let d = h.snapshot(Some(1_000_000 + 10_000), 60);
        assert!(d.full);
        assert_eq!(d.times.len(), 61, "older than the window asked for");
    }

    #[test]
    fn values_are_rounded_for_the_trip_to_the_ui() {
        let mut h = History::default();
        let mut m = metrics(1000, 84.4, false);
        m.net_down = 1234.567;
        h.push(&m);
        let d = h.snapshot(None, 180);
        assert_eq!(d.series["cpu.usage"][0], Some(84.4));
        assert_eq!(d.series["net.down"][0], Some(1235.0));
    }

    #[test]
    fn keeps_three_minutes_and_aligns_every_series() {
        let mut h = History::default();
        for i in 0..250u64 {
            // The GPU only appears for the last 10 samples.
            h.push(&metrics(1_000_000 + i * 1000, i as f32, i >= 240));
        }
        let d = h.snapshot(None, 180);
        assert_eq!(d.times.len(), 181, "180 s window, both ends included");
        assert_eq!(*d.times.last().unwrap(), 1_000_000 + 249_000);
        for (k, v) in &d.series {
            assert_eq!(v.len(), d.times.len(), "{k} is aligned");
        }
        assert_eq!(d.series["cpu.usage"].last().unwrap(), &Some(249.0));
        assert_eq!(d.series["mem.ram"][0], Some(25.0));
        let g = &d.series["gpu.L1.vram"];
        assert_eq!(g.iter().filter(|v| v.is_some()).count(), 10);
        assert_eq!(*g.last().unwrap(), Some(25.0));
        assert_eq!(d.series["gpu.L1.usage"][0], None, "no value before the GPU appeared");
    }

    #[test]
    fn repeated_or_empty_samples_are_ignored() {
        let mut h = History::default();
        h.push(&metrics(5000, 1.0, false));
        h.push(&metrics(5000, 2.0, false));
        h.push(&metrics(4000, 3.0, false));
        h.push(&Metrics { ts_ms: 9000, ..Default::default() }); // cpu_cores == 0: the empty start value
        assert_eq!(h.snapshot(None, 180).times, vec![5000]);
    }

    #[test]
    fn a_clock_set_back_starts_the_history_over() {
        let mut h = History::default();
        for i in 0..10u64 {
            h.push(&metrics(2_000_000 + i * 1000, 1.0, false));
        }
        h.push(&metrics(1_000_000, 2.0, false)); // 20 minutes back
        h.push(&metrics(1_001_000, 3.0, false));
        assert_eq!(h.snapshot(None, 180).times, vec![1_000_000, 1_001_000]);
        // The UI still has the old (later) times: it gets the whole new window.
        assert!(h.snapshot(Some(2_009_000), 180).full);
    }

    #[test]
    fn battery_charge_is_sampled_every_five_seconds() {
        let mut h = History::default();
        for i in 0..20u64 {
            let mut m = metrics(i * 1000, 0.0, false);
            m.battery.present = true;
            m.battery.percent = 80;
            h.push(&m);
        }
        assert_eq!(h.snapshot(None, 180).charge_times, vec![0, 5000, 10000, 15000]);
    }
}
