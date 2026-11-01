//! Opt-in alerts (Settings > Alerts, every one off by default), shown as Windows notifications.
//!
//! Each alert fires once when its condition starts and re-arms only after the value is clearly back (a little
//! below the threshold), so a value wobbling around the line doesn't notify every few seconds. Drive health
//! remembers what it already reported (alerts.json), so a drive that is known to be worn isn't reported at every
//! start. The rules only look at the samples the sampler takes anyway; with an alert on, it also reads drive space,
//! commit charge, drive health and the GPU sensors while the popup is closed (see `Features::alerts`).

use crate::diskhealth::{verdict, DiskHealth};
use crate::hardware::DriveStatic;
use crate::metrics::Metrics;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertConfig {
    /// A drive is fuller than `disk_pct` %.
    pub disk: bool,
    pub disk_pct: u8,
    /// Committed memory above 90 % of its limit for 30 s (programs are about to be refused memory).
    pub memory: bool,
    /// GPU at `gpu_temp` °C or more for 30 s; a drive at its own warning temperature; the CPU (where Windows has
    /// a sensor) at 90 °C for 30 s.
    pub temp: bool,
    pub gpu_temp: u8,
    /// A drive's health verdict got worse (good -> caution -> bad).
    pub health: bool,
    /// One app uses `hog_pct` % of the whole CPU or more for 5 minutes.
    pub hog: bool,
    pub hog_pct: u8,
    /// "tr" or "en": the notifications are written here (the UI may be closed when they go out).
    pub lang: String,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            disk: false,
            disk_pct: 90,
            memory: false,
            temp: false,
            gpu_temp: 85,
            health: false,
            hog: false,
            hog_pct: 50,
            lang: "tr".into(),
        }
    }
}

impl AlertConfig {
    pub fn any(&self) -> bool {
        self.disk || self.memory || self.temp || self.health || self.hog
    }

    pub fn sanitized(mut self) -> Self {
        self.disk_pct = self.disk_pct.clamp(50, 99);
        self.gpu_temp = self.gpu_temp.clamp(60, 105);
        self.hog_pct = self.hog_pct.clamp(10, 100);
        if self.lang != "en" {
            self.lang = "tr".into();
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

const MEMORY_PCT: f64 = 90.0;
const CPU_TEMP: f32 = 90.0;
/// A drive that doesn't say where its warning temperature is.
const DRIVE_TEMP: i16 = 70;
const SUSTAIN: Duration = Duration::from_secs(30);
const HOG_SUSTAIN: Duration = Duration::from_secs(5 * 60);
/// A hogging app has to stay below the line this long before it can be reported again.
const HOG_REARM: Duration = Duration::from_secs(2 * 60);

fn gb(bytes: u64) -> String {
    let g = bytes as f64 / (1u64 << 30) as f64;
    if g >= 1000.0 { format!("{:.1} TB", g / 1024.0) } else if g >= 10.0 { format!("{g:.0} GB") } else { format!("{g:.1} GB") }
}

fn health_rank(status: &str) -> u8 {
    match status {
        "warn" => 1,
        "bad" => 2,
        _ => 0,
    }
}

/// Watches the samples; `check` returns the alerts that start now.
#[derive(Default)]
pub struct Engine {
    disk_fired: HashSet<String>,
    memory_since: Option<Instant>,
    memory_fired: bool,
    /// Per GPU / CPU: above the line since, and fired.
    hot_since: HashMap<String, Instant>,
    hot_fired: HashSet<String>,
    drive_hot_fired: HashSet<u32>,
    hog_since: HashMap<String, Instant>,
    hog_below: HashMap<String, Instant>,
    hog_fired: HashSet<String>,
    /// Drive (model + index) -> the worst status already reported, kept in `store`.
    health_seen: HashMap<String, String>,
    store: Option<PathBuf>,
}

impl Engine {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        let store = data_dir.map(|d| d.join("alerts.json"));
        let health_seen = store
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { store, health_seen, ..Default::default() }
    }

    fn save_health(&self) {
        if let Some(p) = &self.store {
            if let Ok(s) = serde_json::to_string(&self.health_seen) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    pub fn check(&mut self, cfg: &AlertConfig, m: &Metrics, drives: &[DriveStatic], now: Instant) -> Vec<Alert> {
        let tr = cfg.lang != "en";
        let mut out = Vec::new();
        let mut alert = |title: String, body: String| out.push(Alert { title, body });

        // --- drive almost full (C:, D:, ...)
        if cfg.disk {
            for d in m.disks.iter().filter(|d| d.total > (1u64 << 30)) {
                // Rounded down: 99.7 % full with 6 GB left must not read "100 % full".
                let pct = (d.used as f64 / d.total as f64 * 100.0).floor();
                let name = d.mount.trim_end_matches(['\\', '/']).to_string();
                if pct >= cfg.disk_pct as f64 && self.disk_fired.insert(name.clone()) {
                    let free = gb(d.total.saturating_sub(d.used));
                    if tr {
                        alert("Disk dolmak üzere".into(), format!("{name} %{pct:.0} dolu, {free} boş kaldı."));
                    } else {
                        alert("Drive almost full".into(), format!("{name} is {pct:.0}% full, {free} free."));
                    }
                } else if pct < cfg.disk_pct as f64 - 2.0 {
                    self.disk_fired.remove(&name);
                }
            }
        }

        // --- committed memory near its limit
        if cfg.memory && m.commit_limit > 0 {
            let pct = (m.commit_used as f64 / m.commit_limit as f64 * 100.0).floor();
            if pct >= MEMORY_PCT {
                let since = *self.memory_since.get_or_insert(now);
                if !self.memory_fired && now.duration_since(since) >= SUSTAIN {
                    self.memory_fired = true;
                    let (used, limit) = (gb(m.commit_used), gb(m.commit_limit));
                    if tr {
                        alert(
                            "Bellek sınıra yaklaştı".into(),
                            format!("Ayrılan bellek %{pct:.0} ({used} / {limit}). Sınıra gelince programlar yeni bellek alamaz."),
                        );
                    } else {
                        alert(
                            "Memory near its limit".into(),
                            format!("Committed memory is at {pct:.0}% ({used} / {limit}). At the limit programs can't get more."),
                        );
                    }
                }
            } else {
                self.memory_since = None;
                if pct < MEMORY_PCT - 5.0 {
                    self.memory_fired = false;
                }
            }
        }

        // --- temperatures: GPU, CPU (30 s above the line), drives (their own warning temperature)
        if cfg.temp {
            let mut hot: Vec<(String, String, f32, f32)> = Vec::new(); // key, what, value, line
            for g in &m.gpus {
                hot.push((format!("gpu:{}", g.luid), g.name.clone(), g.sensors.temp_c.unwrap_or(f32::NAN), cfg.gpu_temp as f32));
            }
            let cpu = if tr { "İşlemci" } else { "CPU" };
            hot.push(("cpu".into(), cpu.into(), m.cpu_temp_c.unwrap_or(f32::NAN), CPU_TEMP));
            for (key, what, v, line) in hot {
                if v >= line {
                    let since = *self.hot_since.entry(key.clone()).or_insert(now);
                    if now.duration_since(since) >= SUSTAIN && self.hot_fired.insert(key) {
                        if tr {
                            alert(format!("{what} ısındı"), format!("{v:.0} °C (30 saniyedir {line:.0} °C'nin üstünde)."));
                        } else {
                            alert(format!("{what} is running hot"), format!("{v:.0} °C (above {line:.0} °C for 30 seconds)."));
                        }
                    }
                } else {
                    self.hot_since.remove(&key);
                    // NaN (no reading) keeps a fired alert fired; a real reading must be clearly lower to re-arm.
                    if v < line - 5.0 {
                        self.hot_fired.remove(&key);
                    }
                }
            }
            for h in &m.disk_health {
                let Some(t) = h.temp_c else { continue };
                let line = h.warn_c.unwrap_or(DRIVE_TEMP);
                if t >= line && self.drive_hot_fired.insert(h.index) {
                    let model = drive_model(drives, h.index, tr);
                    if tr {
                        alert("Disk ısındı".into(), format!("{model}: {t} °C (diskin uyarı sınırı {line} °C)."));
                    } else {
                        alert("Drive is running hot".into(), format!("{model}: {t} °C (its warning temperature is {line} °C)."));
                    }
                } else if t < line - 3 {
                    self.drive_hot_fired.remove(&h.index);
                }
            }
        }

        // --- drive health got worse (remembered across starts). Only once the drive list is in: a check before it
        // remembered the drive under "Disk 0" and reported it again under its model name seconds later.
        if cfg.health && !drives.is_empty() {
            let mut changed = false;
            for h in &m.disk_health {
                // Wear and errors only: heat is the temperature alert's business (and passes).
                let status = verdict(&DiskHealth { temp_c: None, ..h.clone() });
                if status.is_empty() {
                    continue;
                }
                let model = drive_model(drives, h.index, tr);
                // not the shown name: that one is translated for a drive without a model ("Disk 0" / "Drive 0")
                let raw = drives.iter().find(|d| d.index == Some(h.index)).map(|d| d.model.as_str()).unwrap_or("");
                let key = format!("{raw}#{}", h.index);
                let before = self.health_seen.get(&key).map(|s| health_rank(s)).unwrap_or(0);
                if health_rank(status) > before {
                    let word = match (status, tr) {
                        ("bad", true) => "Kötü",
                        ("bad", false) => "Bad",
                        (_, true) => "Dikkat",
                        (_, false) => "Caution",
                    };
                    let wear = h.nvme.as_ref().map(|n| n.used_pct);
                    let body = match (wear, tr) {
                        (Some(w), true) => {
                            format!("{model}: {word} (ömrünün %{w}'{} kullanıldı). Yedeklerinizi kontrol edin.", tr_possessive(w.into()))
                        }
                        (Some(w), false) => format!("{model}: {word} ({w}% of its life used). Check your backups."),
                        (None, true) => format!("{model}: {word}. Yedeklerinizi kontrol edin."),
                        (None, false) => format!("{model}: {word}. Check your backups."),
                    };
                    alert(if tr { "Disk sağlığı kötüleşti".into() } else { "Drive health got worse".into() }, body);
                }
                if self.health_seen.get(&key).map(String::as_str) != Some(status) {
                    self.health_seen.insert(key, status.to_string());
                    changed = true;
                }
            }
            if changed {
                self.save_health();
            }
        }

        // --- one app hogging the CPU for minutes (only fresh process measurements count)
        if cfg.hog && m.procs_sampled {
            let line = cfg.hog_pct as f32;
            let above: Vec<(String, f32)> =
                m.top_cpu.iter().filter(|p| p.cpu >= line).map(|p| (p.name.clone(), p.cpu)).collect();
            for (name, v) in &above {
                self.hog_below.remove(name);
                let since = *self.hog_since.entry(name.clone()).or_insert(now);
                if now.duration_since(since) >= HOG_SUSTAIN && self.hog_fired.insert(name.clone()) {
                    let app = name.trim_end_matches(".exe");
                    if tr {
                        alert(
                            "Bir uygulama işlemciyi yiyor".into(),
                            format!(
                                "{app} 5 dakikadır işlemcinin %{:.0}'{} kullanıyor.",
                                v,
                                tr_possessive_object(v.round().max(0.0) as u64)
                            ),
                        );
                    } else {
                        alert("An app is hogging the CPU".into(), format!("{app} has used {v:.0}% of the CPU for 5 minutes."));
                    }
                }
            }
            let gone: Vec<String> = self.hog_since.keys().filter(|k| !above.iter().any(|(n, _)| n == *k)).cloned().collect();
            for name in gone {
                self.hog_since.remove(&name);
                let below = *self.hog_below.entry(name.clone()).or_insert(now);
                if now.duration_since(below) >= HOG_REARM {
                    self.hog_fired.remove(&name);
                    self.hog_below.remove(&name);
                }
            }
            // A fired app that dropped out of the list entirely also re-arms after a while.
            let quiet: Vec<String> = self.hog_fired.iter().filter(|n| !above.iter().any(|(a, _)| a == *n)).cloned().collect();
            for name in quiet {
                let below = *self.hog_below.entry(name.clone()).or_insert(now);
                if now.duration_since(below) >= HOG_REARM {
                    self.hog_fired.remove(&name);
                    self.hog_below.remove(&name);
                }
            }
        }
        out
    }
}

/// The Turkish possessive ending after a number as it is read out (like `trPossessive` in format.ts):
/// %2'si (ikisi), %5'i (beşi), %10'u (onu), %40'ı (kırkı), %100'ü (yüzü). A fixed "'i" made "%2'i".
fn tr_possessive(n: u64) -> &'static str {
    const ONES: [&str; 10] = ["", "i", "si", "ü", "ü", "i", "sı", "si", "i", "u"]; // bir iki üç dört beş altı yedi sekiz dokuz
    const TENS: [&str; 10] = ["", "u", "si", "u", "ı", "si", "ı", "i", "i", "ı"]; // on yirmi otuz ... doksan
    if n == 0 {
        return "ı"; // sıfırı
    }
    if n % 10 != 0 {
        return ONES[(n % 10) as usize];
    }
    if n % 100 != 0 {
        return TENS[((n / 10) % 10) as usize];
    }
    let mut zeros = 0;
    let mut m = n;
    while m % 10 == 0 {
        m /= 10;
        zeros += 1;
    }
    match zeros {
        2 => "ü",     // yüzü
        3..=5 => "i", // bini
        6..=8 => "u", // milyonu
        _ => "ı",     // milyarı
    }
}

/// The same ending as the object of a verb: %2'sini, %50'sini, %60'ını, %78'ini, %30'unu.
fn tr_possessive_object(n: u64) -> String {
    let p = tr_possessive(n);
    let vowel = p.chars().last().unwrap_or('i');
    format!("{p}n{vowel}")
}

fn drive_model(drives: &[DriveStatic], index: u32, tr: bool) -> String {
    drives
        .iter()
        .find(|d| d.index == Some(index) && !d.model.is_empty())
        .map(|d| d.model.clone())
        .unwrap_or_else(|| if tr { format!("Disk {index}") } else { format!("Drive {index}") })
}

/// The test notification behind the "Send a test" button.
pub fn test_alert(lang: &str) -> Alert {
    if lang == "en" {
        Alert { title: "PulseTray alerts".into(), body: "Alerts will look like this.".into() }
    } else {
        Alert { title: "PulseTray uyarıları".into(), body: "Uyarılar böyle görünecek.".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diskhealth::NvmeHealth;
    use crate::gpusensors::GpuSensors;
    use crate::metrics::{DiskInfo, GpuNow, ProcInfo};

    const GIB: u64 = 1 << 30;

    fn all_on() -> AlertConfig {
        AlertConfig { disk: true, memory: true, temp: true, health: true, hog: true, ..Default::default() }
    }

    fn base() -> Metrics {
        Metrics { cpu_cores: 12, commit_limit: 48 * GIB, commit_used: 20 * GIB, ..Default::default() }
    }

    fn drive(mount: &str, used_gb: u64, total_gb: u64) -> DiskInfo {
        DiskInfo { mount: mount.into(), total: total_gb * GIB, used: used_gb * GIB, ..Default::default() }
    }

    #[test]
    fn everything_is_off_by_default() {
        let cfg = AlertConfig::default();
        assert!(!cfg.any());
        let mut m = base();
        m.disks = vec![drive("C:\\", 1800, 1863)];
        m.commit_used = 47 * GIB;
        assert!(Engine::default().check(&cfg, &m, &[], Instant::now()).is_empty());
    }

    #[test]
    fn a_full_drive_is_reported_once_and_again_after_it_was_freed() {
        let (cfg, mut e, t) = (all_on(), Engine::default(), Instant::now());
        let mut m = base();
        m.disks = vec![drive("C:\\", 1700, 1863), drive("F:\\", 600, 931)];
        let a = e.check(&cfg, &m, &[], t);
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].title, "Disk dolmak üzere");
        assert!(a[0].body.starts_with("C: %91 dolu"), "{}", a[0].body);
        let mut nearly = base();
        nearly.disks = vec![DiskInfo { mount: "E:\\".into(), total: 1000 * GIB, used: 997 * GIB, ..Default::default() }];
        let b = Engine::default().check(&cfg, &nearly, &[], t);
        assert!(b[0].body.starts_with("E: %99 dolu, 3.0 GB boş"), "rounded down, not 100: {}", b[0].body);
        assert!(e.check(&cfg, &m, &[], t).is_empty(), "only once");
        m.disks[0].used = 1660 * GIB; // 89 %: inside the margin, still armed off
        assert!(e.check(&cfg, &m, &[], t).is_empty());
        m.disks[0].used = 1500 * GIB; // freed
        e.check(&cfg, &m, &[], t);
        m.disks[0].used = 1750 * GIB;
        assert_eq!(e.check(&cfg, &m, &[], t).len(), 1, "full again: reported again");
    }

    #[test]
    fn memory_and_temperatures_need_30_seconds() {
        let (cfg, mut e, t) = (all_on(), Engine::default(), Instant::now());
        let mut m = base();
        m.commit_used = 45 * GIB; // 94 %
        m.gpus = vec![GpuNow {
            luid: "l1".into(),
            name: "NVIDIA GeForce RTX 4070".into(),
            sensors: GpuSensors { temp_c: Some(88.0), ..Default::default() },
            ..Default::default()
        }];
        assert!(e.check(&cfg, &m, &[], t).is_empty(), "not yet");
        let a = e.check(&cfg, &m, &[], t + Duration::from_secs(31));
        let titles: Vec<&str> = a.iter().map(|x| x.title.as_str()).collect();
        assert_eq!(titles, ["Bellek sınıra yaklaştı", "NVIDIA GeForce RTX 4070 ısındı"]);
        assert!(e.check(&cfg, &m, &[], t + Duration::from_secs(60)).is_empty(), "once");
        // a missing reading doesn't re-arm the GPU, a cool one does
        m.gpus[0].sensors.temp_c = None;
        e.check(&cfg, &m, &[], t + Duration::from_secs(61));
        m.gpus[0].sensors.temp_c = Some(88.0);
        assert!(e.check(&cfg, &m, &[], t + Duration::from_secs(100)).is_empty());
        m.gpus[0].sensors.temp_c = Some(70.0);
        e.check(&cfg, &m, &[], t + Duration::from_secs(101));
        m.gpus[0].sensors.temp_c = Some(88.0);
        e.check(&cfg, &m, &[], t + Duration::from_secs(102));
        assert_eq!(e.check(&cfg, &m, &[], t + Duration::from_secs(133)).len(), 1);
    }

    #[test]
    fn drive_health_is_reported_when_it_gets_worse_and_remembered_across_starts() {
        let dir = std::env::temp_dir().join(format!("pulsetray-alerts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = all_on();
        let drives = vec![DriveStatic { model: "Samsung SSD 990 PRO 2TB".into(), index: Some(0), ..Default::default() }];
        let worn = |used: u8| {
            let mut m = base();
            m.disk_health = vec![DiskHealth {
                index: 0,
                nvme: Some(NvmeHealth { used_pct: used, spare_pct: 100, spare_threshold_pct: 10, ..Default::default() }),
                ..Default::default()
            }];
            m
        };
        let mut e = Engine::new(Some(dir.clone()));
        assert!(e.check(&cfg, &worn(2), &drives, Instant::now()).is_empty(), "good: nothing to say");
        let a = e.check(&cfg, &worn(95), &drives, Instant::now());
        assert_eq!(a.len(), 1, "{a:?}");
        assert!(a[0].body.contains("Samsung SSD 990 PRO 2TB: Dikkat (ömrünün %95'i kullanıldı)"), "{}", a[0].body);
        // a restart doesn't report the same state again
        let mut again = Engine::new(Some(dir.clone()));
        assert!(again.check(&cfg, &worn(95), &drives, Instant::now()).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn drive_health_waits_for_the_drive_list_and_is_reported_once() {
        let cfg = all_on();
        let mut m = base();
        m.disk_health = vec![DiskHealth {
            index: 0,
            nvme: Some(NvmeHealth { used_pct: 92, spare_pct: 100, spare_threshold_pct: 10, ..Default::default() }),
            ..Default::default()
        }];
        let mut e = Engine::default();
        // the hardware lookup hasn't finished: no "Disk 0" alert now and a second one under the model name later
        assert!(e.check(&cfg, &m, &[], Instant::now()).is_empty());
        let drives = vec![DriveStatic { model: "WD PC SN740".into(), index: Some(0), ..Default::default() }];
        let a = e.check(&cfg, &m, &drives, Instant::now());
        assert_eq!(a.len(), 1, "{a:?}");
        assert!(a[0].body.contains("WD PC SN740: Dikkat (ömrünün %92'si kullanıldı)"), "{}", a[0].body);
        // switching the language doesn't make it new either
        let en = AlertConfig { lang: "en".into(), ..cfg };
        assert!(e.check(&en, &m, &drives, Instant::now()).is_empty());
    }

    #[test]
    fn turkish_endings_follow_the_number() {
        let cases = [(0, "ı"), (2, "si"), (3, "ü"), (5, "i"), (6, "sı"), (9, "u"), (10, "u"), (30, "u"), (40, "ı"), (50, "si"), (60, "ı"), (80, "i"), (90, "ı"), (95, "i"), (100, "ü"), (1000, "i")];
        for (n, want) in cases {
            assert_eq!(tr_possessive(n), want, "{n}");
        }
        assert_eq!(tr_possessive_object(50), "sini");
        assert_eq!(tr_possessive_object(60), "ını");
        assert_eq!(tr_possessive_object(30), "unu");
        assert_eq!(tr_possessive_object(78), "ini");
        assert_eq!(tr_possessive_object(84), "ünü");
    }

    #[test]
    fn a_hogging_app_needs_five_minutes_of_fresh_measurements() {
        let (cfg, mut e, t) = (all_on(), Engine::default(), Instant::now());
        let mut m = base();
        m.procs_sampled = true;
        m.top_cpu = vec![ProcInfo { name: "vmmem".into(), cpu: 78.0, ..Default::default() }];
        assert!(e.check(&cfg, &m, &[], t).is_empty());
        assert!(e.check(&cfg, &m, &[], t + Duration::from_secs(200)).is_empty());
        let a = e.check(&cfg, &m, &[], t + Duration::from_secs(301));
        assert_eq!(a, vec![Alert {
            title: "Bir uygulama işlemciyi yiyor".into(),
            body: "vmmem 5 dakikadır işlemcinin %78'ini kullanıyor.".into()
        }]);
        assert!(e.check(&cfg, &m, &[], t + Duration::from_secs(900)).is_empty(), "once while it keeps going");
        // calm for 2 minutes, then hogging for 5 minutes again
        m.top_cpu[0].cpu = 5.0;
        e.check(&cfg, &m, &[], t + Duration::from_secs(901));
        e.check(&cfg, &m, &[], t + Duration::from_secs(1030));
        m.top_cpu[0].cpu = 80.0;
        e.check(&cfg, &m, &[], t + Duration::from_secs(1031));
        assert_eq!(e.check(&cfg, &m, &[], t + Duration::from_secs(1332)).len(), 1);
        // stale lists (no new measurement this tick) don't count
        m.procs_sampled = false;
        let mut fresh = Engine::default();
        fresh.check(&cfg, &m, &[], t);
        assert!(fresh.check(&cfg, &m, &[], t + Duration::from_secs(400)).is_empty());
    }

    #[test]
    fn english_texts() {
        let cfg = AlertConfig { lang: "en".into(), ..all_on() };
        let mut m = base();
        m.disks = vec![drive("D:\\", 950, 1000)];
        let a = Engine::default().check(&cfg, &m, &[], Instant::now());
        assert_eq!(a[0].title, "Drive almost full");
        assert!(a[0].body.starts_with("D: is 95% full"), "{}", a[0].body);
    }
}
