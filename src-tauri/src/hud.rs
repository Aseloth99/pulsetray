//! Desktop mini gauge ("Mini gösterge"): a small always-on-top window with a few live numbers.
//!
//! The settings live in the main window (it is the only one that writes them); it sends them here with
//! `set_hud`, and the tray menu can switch the gauge on and off. The window never takes focus (it is a tool
//! window that doesn't activate), can be dragged unless it is locked (then clicks pass through to what is
//! under it), hides while a full-screen app is in front, and remembers where it was put (`hud.json`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "hud";
pub const MIN_OPACITY: f32 = 0.85;
pub const ITEMS: [&str; 6] = ["fps", "cpu", "ram", "gpu", "net", "disk"];
const FILE: &str = "hud.json";
/// How much of the gauge must still be on a monitor for a saved position to be reused (px).
const MIN_VISIBLE: i32 = 40;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HudConfig {
    pub on: bool,
    pub items: Vec<String>,
    /// Card opacity, 0.85 ... 1. More see-through than that, whatever moved behind the card (a chat scrolling,
    /// a video) showed through and made it look as if it flickered and changed colour.
    pub opacity: f32,
    /// Clicks pass through (the gauge can't be dragged while locked).
    pub locked: bool,
    pub hide_fullscreen: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            on: false,
            items: ["cpu", "ram", "gpu", "net"].map(String::from).to_vec(),
            opacity: 0.9,
            locked: false,
            hide_fullscreen: true,
        }
    }
}

impl HudConfig {
    /// Known items only, each once, in the canonical order; opacity in range.
    pub fn sanitized(mut self) -> Self {
        self.items = ITEMS.iter().filter(|k| self.items.iter().any(|i| i == *k)).map(|k| k.to_string()).collect();
        self.opacity = if self.opacity.is_finite() { self.opacity.clamp(MIN_OPACITY, 1.0) } else { 0.9 };
        self
    }

    pub fn shows(&self, item: &str) -> bool {
        self.items.iter().any(|i| i == item)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

/// A monitor's work area in physical pixels: (x, y, width, height).
pub type Area = (i32, i32, u32, u32);

pub fn load_pos(dir: Option<&Path>) -> Option<Pos> {
    let text = std::fs::read_to_string(dir?.join(FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_pos(dir: Option<&PathBuf>, pos: Pos) {
    if let Some(dir) = dir {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(text) = serde_json::to_string(&pos) {
            let _ = std::fs::write(dir.join(FILE), text);
        }
    }
}

/// First place: the top-right corner of the (primary) work area, `margin` px in.
pub fn default_pos(work: Area, size: (u32, u32), margin: i32) -> Pos {
    let (x, y, w, _h) = work;
    Pos { x: x + w as i32 - size.0 as i32 - margin, y: y + margin }
}

/// Is enough of the gauge on one of the monitors? (A saved spot may be on a monitor that was unplugged.)
pub fn on_screen(pos: Pos, size: (u32, u32), areas: &[Area]) -> bool {
    areas.iter().any(|&(x, y, w, h)| {
        let left = pos.x.max(x);
        let right = (pos.x + size.0 as i32).min(x + w as i32);
        let top = pos.y.max(y);
        let bottom = (pos.y + size.1 as i32).min(y + h as i32);
        right - left >= MIN_VISIBLE && bottom - top >= MIN_VISIBLE.min(size.1 as i32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_known_items_in_order_and_clamps_opacity() {
        let c = HudConfig {
            items: ["net", "bogus", "cpu", "net"].map(String::from).to_vec(),
            opacity: 7.0,
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.items, vec!["cpu", "net"]);
        assert_eq!(c.opacity, 1.0);
        assert!(c.shows("cpu") && !c.shows("gpu"));
        assert_eq!(HudConfig { opacity: f32::NAN, ..Default::default() }.sanitized().opacity, 0.9);
        assert_eq!(HudConfig { opacity: 0.0, ..Default::default() }.sanitized().opacity, MIN_OPACITY);
        // a 60 % card from an older version becomes the least see-through choice left
        assert_eq!(HudConfig { opacity: 0.6, ..Default::default() }.sanitized().opacity, MIN_OPACITY);
    }

    #[test]
    fn config_from_the_ui_fills_missing_fields() {
        let c: HudConfig = serde_json::from_str(r#"{"on":true}"#).unwrap();
        assert!(c.on && c.hide_fullscreen && !c.locked);
        assert_eq!(c.items.len(), 4);
    }

    #[test]
    fn default_place_is_the_top_right_corner() {
        // 1920x1040 work area (taskbar at the bottom), second monitor to the left.
        assert_eq!(default_pos((0, 0, 1920, 1040), (200, 120), 16), Pos { x: 1704, y: 16 });
        assert_eq!(default_pos((-1280, 0, 1280, 1024), (200, 120), 16), Pos { x: -216, y: 16 });
    }

    #[test]
    fn saved_place_on_a_missing_monitor_is_not_reused() {
        let areas = [(0, 0, 1920, 1040)];
        assert!(on_screen(Pos { x: 1704, y: 16 }, (200, 120), &areas));
        assert!(on_screen(Pos { x: 1900, y: 1000 }, (200, 120), &areas) == false, "only 20 px visible");
        assert!(!on_screen(Pos { x: -900, y: 100 }, (200, 120), &areas), "monitor on the left was unplugged");
        assert!(on_screen(Pos { x: -900, y: 100 }, (200, 120), &[(-1280, 0, 1280, 1024), (0, 0, 1920, 1040)]));
    }

    #[test]
    fn position_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("pulsetray-hud-test-{}", std::process::id()));
        save_pos(Some(&dir), Pos { x: -12, y: 345 });
        assert_eq!(load_pos(Some(&dir)), Some(Pos { x: -12, y: 345 }));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_pos(Some(&dir)), None);
    }
}
