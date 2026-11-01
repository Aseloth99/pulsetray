//! Per-day network totals, persisted so "today" survives restarts.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use windows::Win32::System::SystemInformation::GetLocalTime;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct DayUsage {
    pub date: String,
    pub rx: u64,
    pub tx: u64,
}

pub struct UsageStore {
    path: Option<PathBuf>,
    pub today: DayUsage,
    dirty: bool,
}

pub fn today_string() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

impl UsageStore {
    pub fn load(dir: Option<PathBuf>) -> Self {
        let path = dir.map(|d| {
            let _ = std::fs::create_dir_all(&d);
            d.join("usage.json")
        });
        let stored: Option<DayUsage> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok());
        let date = today_string();
        let today = match stored {
            Some(d) if d.date == date => d,
            _ => DayUsage { date, rx: 0, tx: 0 },
        };
        Self { path, today, dirty: false }
    }

    pub fn add(&mut self, rx: u64, tx: u64) {
        let date = today_string();
        if self.today.date != date {
            self.today = DayUsage { date, rx: 0, tx: 0 };
        }
        self.today.rx += rx;
        self.today.tx += tx;
        self.dirty = true;
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if let (Some(p), Ok(s)) = (&self.path, serde_json::to_string(&self.today)) {
            let _ = std::fs::write(p, s);
        }
        self.dirty = false;
    }
}
