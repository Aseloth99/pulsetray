mod apps;
mod alerts;
mod autostart;
mod battery;
mod batteryreport;
mod diskhealth;
mod extsensors;
mod fps;
mod fullscreen;
mod gpusensors;
mod hardware;
mod history;
mod hud;
mod metrics;
mod pdh;
mod ping;
mod pingstate;
mod procs;
mod remote_notice;
mod toast;
mod trayicon;
mod updater;
mod updatestate;
mod usage;
mod wifi;

use hardware::Hardware;
use history::{History, HistoryDto, TopAppsDto};
use metrics::{Features, Metrics, Sampler};
use pingstate::{PingSnapshot, PingState, PingStatus};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Rect, State, WebviewWindow,
    WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const MARGIN: f64 = 8.0;
const DEFAULT_W: u32 = 564;
/// Popup height before the UI has measured its content (logical pixels).
const DEFAULT_H: u32 = 744;
/// Ignore a tray click that arrives right after a blur-hide, otherwise
/// clicking the tray icon while the popup is open would hide and instantly re-show it.
const REOPEN_GUARD_MS: u64 = 250;
/// A freshly shown popup stays invisible (cloaked) while its page re-measures the tiles, so it never visibly changes
/// size right after it appears. If the page doesn't report back in this time it is revealed anyway.
const SETTLE_MAX: Duration = Duration::from_millis(250);
/// Showing the desktop gauge can move the keyboard focus for a moment; a blur of the popup this soon after it is not
/// "the user clicked elsewhere".
const HUD_FOCUS_GUARD_MS: u64 = 700;
/// Hidden this long, the web view is asked to give its memory back (reopening it right away stays instant).
const TRIM_AFTER: Duration = Duration::from_secs(60);
/// When this process started: a clean exit only proves the running version once it has been up for a while.
static STARTED: OnceLock<Instant> = OnceLock::new();
/// The app is exiting to install an update: the exe on disk is already the NEW one, so this exit must not "confirm" it.
static EXITING_FOR_UPDATE: AtomicBool = AtomicBool::new(false);

/// The right-click menu is a small window of its own (the native Windows menu can't follow the app's look).
const MENU_LABEL: &str = "traymenu";
/// Menu window size before the UI has measured its content (logical pixels).
const MENU_DEFAULT_W: u32 = 196;
const MENU_DEFAULT_H: u32 = 236;
/// Transparent frame around the menu card (room for its shadow), logical pixels. Must match `.shell` in menu/+page.svelte.
const MENU_PAD: f64 = 12.0;

struct AppState {
    latest: Mutex<Metrics>,
    hardware: Mutex<Option<Hardware>>,
    last_blur_hide: AtomicU64,
    /// The popup was shown cloaked while its page re-measures the layout: this show's token (ms) until `popup_settled`
    /// (or `SETTLE_MAX`) reveals it; 0 = nothing pending.
    revealing: AtomicU64,
    /// A popup blur before this time (ms since epoch) is ignored: the desktop gauge was just shown (see `apply_hud`).
    blur_guard_until: AtomicU64,
    /// Requested popup width in logical pixels (the UI asks for more room in advanced mode).
    popup_width: AtomicU32,
    /// Height that fits the tiles on screen (the UI measures its content and reports it).
    popup_height: AtomicU32,
    /// "Install updates automatically" setting, pushed from the UI (default on): download in the background and
    /// install at a quiet moment. Off = an update is only offered (pill, tray menu, update screen).
    auto_update: AtomicBool,
    /// "Check for Updates Automatically" setting, pushed from the UI (default on): off = no daily background check, and
    /// the automatic install (above) waits for the check too. The manual "Check" always works.
    auto_check: AtomicBool,
    /// UI language ("tr" / "en"), pushed from the UI; picks the announcement text.
    lang: Mutex<String>,
    /// Latest announcement from the server (the UI asks for it when it loads and also gets an event).
    notice: Mutex<Option<remote_notice::Notice>>,
    /// The update on offer and what is happening to it (updatestate.rs): one download at a time, shared by the
    /// daily check, the automatic mode, the update screen and the tray menu.
    update: updatestate::UpdateState,
    /// Started by an update (`--updated <old version>`): the "PulseTray was updated" notification is still to come.
    updated_from: Mutex<Option<String>>,
    /// Pinned popup: stays open when it loses focus and can be dragged.
    pinned: AtomicBool,
    /// The user dragged the popup to a size of their own (remembered); it then stops auto-fitting the content.
    custom_size: AtomicBool,
    /// The next `set_popup_height` must resize the window even if the number is unchanged (the window was dragged
    /// to another size in between, so the remembered number no longer describes it).
    refit_height: AtomicBool,
    /// When the user pressed an edge grip (ms since epoch; 0 = not resizing). Dragging an edge takes focus away
    /// from the popup for a moment, which must not count as "clicked elsewhere".
    resizing_since: AtomicU64,
    /// Where the user left a pinned popup, so hiding and showing it again doesn't move it.
    pinned_pos: Mutex<Option<PhysicalPosition<i32>>>,
    /// Where `place_popup` last put the popup: a window still there has not been moved by the user, so a size change
    /// places it by the same rule again (next to the tray icon) instead of keeping a corner.
    auto_pos: Mutex<Option<PhysicalPosition<i32>>>,
    /// Tray icon centre (physical px) the popup was last anchored to, so a resize can re-place it.
    anchor: Mutex<Option<(f64, f64)>>,
    /// The WebView2 runtime has been told to give memory back (popup hidden for a while).
    trimmed: AtomicBool,
    /// Which tiles are on screen right now (pushed from the UI); only those are measured.
    features: Mutex<Features>,
    /// Latency targets, "gateway" or hosts / IPv4 addresses (pushed from the UI).
    ping_targets: Mutex<Vec<String>>,
    /// Latency test: running / last run / graphs.
    ping: Mutex<PingState>,
    /// The popup's UI has loaded and told us its width (the first automatic show waits for this, otherwise the
    /// popup opens at the default width and, in Advanced mode, in a single column).
    ui_ready: AtomicBool,
    /// Right-click menu window size in logical pixels (the menu page measures itself and reports it).
    menu_width: AtomicU32,
    menu_height: AtomicU32,
    /// Where the menu was opened (the pointer, physical px): a size report that arrives while it is open re-places it.
    menu_cursor: Mutex<Option<(f64, f64)>>,
    /// The menu's web view has been told to give memory back (menu hidden for a while).
    menu_trimmed: AtomicBool,
    /// Chart history, also recorded while the popup is closed (see history.rs).
    history: Mutex<History>,
    /// What the tray icon shows (trayicon::Mode as u8): the logo, or a live CPU / GPU / RAM number.
    tray_mode: AtomicU8,
    /// Desktop mini gauge settings (pushed from the main window; the tray menu toggles `on`).
    hud: Mutex<hud::HudConfig>,
    /// The gauge is hidden right now because a full-screen app is in front.
    hud_fs_hidden: AtomicBool,
    /// Where the gauge was dragged to; the sampler loop saves it to hud.json when it changes.
    hud_pos: Mutex<Option<hud::Pos>>,
    hud_pos_dirty: AtomicBool,
    /// Which alerts are on (Settings > Alerts; all off by default), pushed from the UI.
    alerts: Mutex<alerts::AlertConfig>,
    /// The last battery report (powercfg takes a second or two; the history changes slowly).
    battery_report: Mutex<Option<(Instant, batteryreport::BatteryHistory)>>,
    /// FPS of the app in front for the mini gauge (a trace session, only while the gauge shows FPS).
    fps: Mutex<fps::FpsMeter>,
}

type AppStateRef<'a> = State<'a, Arc<AppState>>;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn popup_visible(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false))
}

// ------------------------------------------------------------------ simple state commands

#[tauri::command]
fn get_metrics(state: AppStateRef<'_>) -> Metrics {
    state.latest.lock().unwrap().clone()
}

/// The charts' last `secs` seconds; with `since` (the UI's newest sample) only what it missed while the popup
/// was closed. The popup asks whenever it opens.
#[tauri::command]
fn get_history(state: AppStateRef<'_>, since: Option<u64>, secs: Option<u64>) -> HistoryDto {
    state.history.lock().unwrap().snapshot(since, secs.unwrap_or(180))
}

/// Paths and icons of the apps in the "Top apps" list: (name, a few of its pids) per app the UI has no icon for yet.
#[tauri::command]
async fn app_infos(apps: Vec<(String, Vec<u32>)>) -> Vec<apps::AppInfo> {
    // Off the main thread: reading icons out of exe files takes a few milliseconds each.
    tauri::async_runtime::spawn_blocking(move || apps::infos(&apps)).await.unwrap_or_default()
}

/// "Open file location" in the app list's menu.
#[tauri::command]
fn reveal_app(pids: Vec<u32>) -> bool {
    apps::reveal(&pids)
}

/// "End task" in the app list's menu (after the user confirmed it).
#[tauri::command]
fn end_app(name: String, pids: Vec<u32>) -> apps::EndResult {
    apps::end(&name, &pids)
}

/// Tray icon: "logo" or a live "cpu" / "gpu" / "ram" number (Settings > Appearance).
#[tauri::command]
fn set_tray_icon(state: AppStateRef<'_>, mode: String) {
    state.tray_mode.store(trayicon::Mode::parse(&mode).as_u8(), Ordering::Relaxed);
}

#[tauri::command]
fn get_hardware(state: AppStateRef<'_>) -> Option<Hardware> {
    state.hardware.lock().unwrap().clone()
}

/// Who used the most CPU / memory / disk / GPU at chart time `t` (the charts' "who caused this peak?" readout).
#[tauri::command]
fn top_apps_at(state: AppStateRef<'_>, t: u64, kind: String) -> Option<TopAppsDto> {
    state.history.lock().unwrap().tops_at(t, &kind)
}

#[tauri::command]
fn set_alerts(state: AppStateRef<'_>, config: alerts::AlertConfig) {
    *state.alerts.lock().unwrap() = config.sanitized();
}

/// Settings > Alerts > "Send a test": Windows shows it (or the error is shown under the button).
#[tauri::command]
async fn alert_test(app: AppHandle, lang: String) -> Result<(), String> {
    let a = alerts::test_alert(&lang);
    let data_dir = app.path().app_data_dir().ok();
    let h = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        toast::Toaster::new(&toast::app_id(&h), data_dir).show(&a.title, &a.body, {
            let h = h.clone();
            move || open_from_toast(&h)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Battery history page: Windows' battery report, at most every 10 minutes unless `force`.
/// Errors: "no_battery" (a PC without one) or powercfg's own message.
#[tauri::command]
async fn battery_history(app: AppHandle, force: bool) -> Result<batteryreport::BatteryHistory, String> {
    let state = app.state::<Arc<AppState>>().inner().clone();
    if !force {
        if let Some((at, h)) = state.battery_report.lock().unwrap().as_ref() {
            if at.elapsed() < Duration::from_secs(600) {
                return Ok(h.clone());
            }
        }
    }
    let h = tauri::async_runtime::spawn_blocking(|| batteryreport::generate(14).and_then(|x| batteryreport::parse(&x)))
        .await
        .map_err(|e| e.to_string())??;
    *state.battery_report.lock().unwrap() = Some((Instant::now(), h.clone()));
    Ok(h)
}

/// A click on a notification that is still on screen opens the popup.
fn open_from_toast(app: &AppHandle) {
    let h = app.clone();
    let _ = app.run_on_main_thread(move || show_popup(&h, None));
}

#[tauri::command]
fn set_features(
    state: AppStateRef<'_>,
    processes: bool,
    gpu: bool,
    battery: bool,
    wifi: bool,
    ping: bool,
) {
    *state.features.lock().unwrap() = Features { processes, gpu, battery, wifi, ping, alerts: false };
}

#[tauri::command]
fn set_auto_update(state: AppStateRef<'_>, enabled: bool) {
    state.auto_update.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
fn set_auto_check(state: AppStateRef<'_>, enabled: bool) {
    state.auto_check.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
fn get_notice(state: AppStateRef<'_>) -> Option<remote_notice::Notice> {
    state.notice.lock().unwrap().clone()
}

/// Opens a link from an announcement or the help line in the default browser. Only https links on
/// macik.com.tr are accepted (the same rule as for update downloads).
#[tauri::command]
fn open_site(url: String) -> bool {
    if !updater::validate_url(&url) {
        return false;
    }
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide: Vec<u16> = url.trim().encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), None, None, SW_SHOWNORMAL) };
    r.0 as isize > 32
}

#[tauri::command]
fn hide_popup(window: WebviewWindow) {
    let _ = window.hide();
    set_cloak(&window, false);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// The UI language, pushed from the UI (picks the announcement text).
#[tauri::command]
fn set_language(state: AppStateRef<'_>, lang: String) {
    let tr = lang.to_lowercase().starts_with("tr");
    *state.lang.lock().unwrap() = if tr { "tr".into() } else { "en".into() };
}

// ------------------------------------------------------------------ start at login

#[tauri::command]
fn autostart_get(app: AppHandle) -> bool {
    autostart::is_enabled(&app.config().identifier)
}

/// Returns the state afterwards (false if the registry refused).
#[tauri::command]
fn autostart_set(app: AppHandle, enabled: bool) -> Result<bool, String> {
    autostart::set_enabled(&app.config().identifier, enabled)
}

// ------------------------------------------------------------------ latency test

/// What the UI receives after every tick: the test status plus the newest samples.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PingEvent {
    status: PingStatus,
    samples: Vec<ping::PingSample>,
}

fn emit_ping(app: &AppHandle, state: &AppState, samples: Vec<ping::PingSample>) {
    let status = state.ping.lock().unwrap().status(Instant::now());
    let _ = app.emit("ping", &PingEvent { status, samples });
}

#[tauri::command]
fn set_ping_targets(app: AppHandle, state: AppStateRef<'_>, targets: Vec<String>) {
    // Hostnames / IPv4 only, a handful at most; anything else could never be resolved anyway.
    let mut clean: Vec<String> = Vec::new();
    for t in targets.into_iter().map(|t| t.trim().to_lowercase()) {
        let ok = !t.is_empty()
            && t.len() <= 253
            && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if ok && !clean.contains(&t) && clean.len() < 6 {
            clean.push(t);
        }
    }
    state.ping.lock().unwrap().sync_targets(&clean);
    *state.ping_targets.lock().unwrap() = clean;
    emit_ping(&app, &state, Vec::new());
}

/// Start a latency test: `secs` = 60 / 300 / ..., `None` = until stopped. Previous graphs are cleared.
/// Refuses while a full-screen app is in the foreground (the test must never disturb a game).
#[tauri::command]
fn ping_start(app: AppHandle, state: AppStateRef<'_>, secs: Option<u64>) -> Result<(), String> {
    if fullscreen::is_fullscreen_app() {
        return Err("fullscreen".into());
    }
    let targets = state.ping_targets.lock().unwrap().clone();
    {
        let mut p = state.ping.lock().unwrap();
        p.sync_targets(&targets);
        p.start(Instant::now(), secs.map(|s| s.clamp(5, 24 * 3600)));
    }
    emit_ping(&app, &state, Vec::new());
    Ok(())
}

#[tauri::command]
fn ping_stop(app: AppHandle, state: AppStateRef<'_>) {
    state.ping.lock().unwrap().end("stopped", updater::now_secs());
    emit_ping(&app, &state, Vec::new());
}

/// "Continuous" mode (advanced view): measure whenever the popup is open.
#[tauri::command]
fn set_ping_continuous(app: AppHandle, state: AppStateRef<'_>, on: bool) {
    state.ping.lock().unwrap().set_continuous(on, updater::now_secs());
    emit_ping(&app, &state, Vec::new());
}

#[tauri::command]
fn get_ping_state(state: AppStateRef<'_>) -> PingSnapshot {
    let targets = state.ping_targets.lock().unwrap().clone();
    let mut p = state.ping.lock().unwrap();
    p.sync_targets(&targets);
    p.snapshot(Instant::now())
}

/// Pings run only when there is a reason: a user-started test (even with the popup closed), or
/// continuous mode while the popup is open. Never while a full-screen app is in front.
fn spawn_ping_thread(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || loop {
        if state.ping.lock().unwrap().expire(Instant::now(), updater::now_secs()) {
            emit_ping(&app, &state, Vec::new());
        }
        let enabled = state.features.lock().unwrap().ping;
        let (running, continuous) = {
            let p = state.ping.lock().unwrap();
            (p.is_running(), p.continuous)
        };
        if !enabled || !(running || (continuous && popup_visible(&app))) {
            state.ping.lock().unwrap().paused = false;
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        if fullscreen::is_fullscreen_app() {
            let changed = {
                let mut p = state.ping.lock().unwrap();
                let was_paused = p.paused;
                if running {
                    // A test never runs over a game: it ends here and says why.
                    p.end("fullscreen", updater::now_secs());
                    p.paused = false;
                    true
                } else {
                    // Continuous mode just waits until the full-screen app is gone.
                    p.paused = true;
                    !was_paused
                }
            };
            if changed {
                emit_ping(&app, &state, Vec::new());
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        let was_paused = std::mem::replace(&mut state.ping.lock().unwrap().paused, false);
        if was_paused {
            emit_ping(&app, &state, Vec::new());
        }

        let started = Instant::now();
        let targets = state.ping_targets.lock().unwrap().clone();
        let samples = ping::ping_all(&targets);
        state.ping.lock().unwrap().record(&samples);
        emit_ping(&app, &state, samples);
        if let Some(rest) = Duration::from_millis(1000).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    });
}

// ------------------------------------------------------------------ popup window

/// Pinned popups are resized by the user; this restores a remembered size (clamped to the screen).
#[tauri::command]
fn resize_popup(window: WebviewWindow, state: AppStateRef<'_>, width: f64, height: f64) {
    // Not while the user drags an edge: a size set by the page now would fight the drag.
    if !size_is_fixed(&state) || is_resizing(&state) {
        return;
    }
    let (mut w, mut h) = (width.max(240.0), height.max(320.0));
    if let Ok(Some(mon)) = window.current_monitor() {
        let scale = mon.scale_factor();
        w = w.min(mon.size().width as f64 / scale);
        h = h.min(mon.size().height as f64 / scale);
    }
    let before = window.outer_position().ok().zip(window.outer_size().ok());
    let _ = window.set_size(LogicalSize::new(w, h));
    // The size follows the mode (classic / advanced) or a fit, not the user's hand: the window must not end up
    // floating mid-screen because its top-left corner stayed put while the bottom or right edge moved.
    let Some((pos, old)) = before else { return };
    // Still where the app put it (next to the tray icon): place it by the same rule for the new size.
    let auto = *state.auto_pos.lock().unwrap();
    let anchor = *state.anchor.lock().unwrap();
    if let (Some(auto), Some(anchor)) = (auto, anchor) {
        if (auto.x - pos.x).abs() <= 4 && (auto.y - pos.y).abs() <= 4 {
            place_popup(&window, &state, anchor, true);
            return;
        }
    }
    let Ok(new) = window.outer_size() else { return };
    let centre = (pos.x as f64 + old.width as f64 / 2.0, pos.y as f64 + old.height as f64 / 2.0);
    let Ok(Some(mon)) = window.app_handle().monitor_from_point(centre.0, centre.1) else { return };
    let wa = mon.work_area();
    let area = (wa.position.x, wa.position.y, wa.position.x + wa.size.width as i32, wa.position.y + wa.size.height as i32);
    let tol = ((MARGIN + 16.0) * mon.scale_factor()).round() as i32;
    let anchor_below = state.anchor.lock().unwrap().is_none_or(|(_, ay)| ay > (area.1 + area.3) as f64 / 2.0);
    let want = keep_edges(
        area,
        (pos.x, pos.y),
        (old.width as i32, old.height as i32),
        (new.width as i32, new.height as i32),
        tol,
        anchor_below,
    );
    if want != (pos.x, pos.y) {
        let _ = window.set_position(PhysicalPosition::new(want.0, want.1));
    }
}

/// Top-left corner for a window that changes from the `old` to the `new` size (all physical pixels, `area` =
/// left, top, right, bottom of the work area). A window that sat against the right or bottom edge of the screen
/// (within `tol`) keeps that edge, so the popup next to the tray stays next to the tray when it gets smaller or
/// larger; any other window keeps its top-left corner. Always inside the work area afterwards.
/// When both opposite edges are close, the bottom one wins if `anchor_below` (the tray is at the bottom), the left
/// and top ones otherwise.
fn keep_edges(
    area: (i32, i32, i32, i32),
    pos: (i32, i32),
    old: (i32, i32),
    new: (i32, i32),
    tol: i32,
    anchor_below: bool,
) -> (i32, i32) {
    let axis = |lo: i32, hi: i32, p: i32, o: i32, n: i32, prefer_high: bool| {
        let near_lo = p - lo <= tol;
        let near_hi = hi - (p + o) <= tol;
        let start = if near_hi && (!near_lo || prefer_high) { p + o - n } else { p };
        start.clamp(lo, (hi - n).max(lo))
    };
    (
        axis(area.0, area.2, pos.0, old.0, new.0, false),
        axis(area.1, area.3, pos.1, old.1, new.1, anchor_below),
    )
}

/// A pinned popup, or one the user has resized, keeps its size instead of fitting the content.
fn size_is_fixed(state: &AppState) -> bool {
    state.pinned.load(Ordering::Relaxed) || state.custom_size.load(Ordering::Relaxed)
}

/// While an edge drag runs (`start_resize`) the popup must not close when the focus moves. A drag whose end is never
/// seen expires after a minute.
fn is_resizing(state: &AppState) -> bool {
    let since = state.resizing_since.load(Ordering::Relaxed);
    since != 0 && now_ms().saturating_sub(since) < 60_000
}

/// Is the primary mouse button held down right now (whatever window the pointer is over)?
#[cfg(windows)]
fn primary_button_down() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
        fn GetSystemMetrics(index: i32) -> i32;
    }
    const VK_LBUTTON: i32 = 0x01;
    const VK_RBUTTON: i32 = 0x02;
    const SM_SWAPBUTTON: i32 = 23;
    // GetAsyncKeyState reads the physical buttons: with swapped buttons the primary one is the right one.
    unsafe {
        let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        GetAsyncKeyState(key) < 0
    }
}

#[cfg(not(windows))]
fn primary_button_down() -> bool {
    false
}

/// A press on the header of a pinned popup: move the window with the system's own move loop. This is what
/// `data-tauri-drag-region` asks tao for as well, but tao hands the cursor position to WM_NCLBUTTONDOWN as a pointer
/// instead of packed coordinates, and here the window then did not move at all (the same message posted by hand with
/// packed coordinates moves it).
#[tauri::command]
fn start_move(window: WebviewWindow) {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn GetCursorPos(p: *mut Point) -> i32;
            fn ReleaseCapture() -> i32;
            fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        }
        const WM_NCLBUTTONDOWN: u32 = 0x00A1;
        const HTCAPTION: usize = 2;
        let Ok(hwnd) = window.hwnd() else { return };
        let mut p = Point { x: 0, y: 0 };
        unsafe {
            if GetCursorPos(&mut p) == 0 {
                return;
            }
            // lParam of a non-client mouse message: x in the low word, y in the high word (16-bit, signed).
            let packed = (((p.y as i16) as u16 as isize) << 16) | ((p.x as i16) as u16 as isize);
            ReleaseCapture();
            PostMessageW(hwnd.0 as isize, WM_NCLBUTTONDOWN, HTCAPTION, packed);
        }
    }
    #[cfg(not(windows))]
    let _ = window.start_dragging();
}

/// An edge grip was pressed: start the resize here, in one step. The page used to send two calls (a "resizing" flag and
/// the drag itself); Windows shuffles the focus as the size loop starts, that often won the race, and the popup closed
/// (as "clicked elsewhere") instead of resizing. The flag now goes up first and stays up until the mouse button is
/// released; then the focus comes back and the page is told (`resize-ended`).
#[tauri::command]
fn start_resize(app: AppHandle, window: WebviewWindow, state: AppStateRef<'_>, dir: String) {
    use tauri_runtime::ResizeDirection as D;
    let direction = match dir.as_str() {
        "North" => D::North,
        "South" => D::South,
        "East" => D::East,
        "West" => D::West,
        "NorthEast" => D::NorthEast,
        "NorthWest" => D::NorthWest,
        "SouthEast" => D::SouthEast,
        "SouthWest" => D::SouthWest,
        _ => return,
    };
    let token = now_ms().max(1);
    state.resizing_since.store(token, Ordering::Relaxed);
    let as_webview: &tauri::Webview = window.as_ref();
    if as_webview.window().start_resize_dragging(direction).is_err() {
        state.resizing_since.store(0, Ordering::Relaxed);
        return;
    }
    let st = app.state::<Arc<AppState>>().inner().clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(40));
            if st.resizing_since.load(Ordering::Relaxed) != token {
                return; // another resize took over
            }
            if !primary_button_down() {
                break;
            }
        }
        if st.resizing_since.compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let _ = app.emit("resize-ended", ());
            if window.is_visible().unwrap_or(false) {
                // Take focus back so that a later click elsewhere still closes the popup.
                let _ = window.set_focus();
            }
        }
    });
}

/// Tells Rust whether the user has a remembered size for the current mode (see `size_is_fixed`).
#[tauri::command]
fn set_custom_size(state: AppStateRef<'_>, custom: bool) {
    if state.custom_size.swap(custom, Ordering::Relaxed) && !custom {
        state.refit_height.store(true, Ordering::Relaxed);
    }
}

/// The popup is as tall as its content (no empty strip under the tiles); a resized popup keeps the user's size.
#[tauri::command]
fn set_popup_height(window: WebviewWindow, state: AppStateRef<'_>, height: u32) {
    let height = height.clamp(240, 1400);
    let unchanged = state.popup_height.swap(height, Ordering::Relaxed) == height;
    if size_is_fixed(&state) || !window.is_visible().unwrap_or(false) {
        return;
    }
    if is_resizing(&state) {
        // The user is dragging an edge: a refit now would fight the drag. If the drag ends without a size of the
        // user's own (a plain click on a grip), the next measurement fits the content again.
        state.refit_height.store(true, Ordering::Relaxed);
        return;
    }
    if unchanged && !state.refit_height.swap(false, Ordering::Relaxed) {
        return;
    }
    state.refit_height.store(false, Ordering::Relaxed);
    let anchor = *state.anchor.lock().unwrap();
    if let Some(a) = anchor {
        place_popup(&window, &state, a, false);
    }
}

#[tauri::command]
fn set_popup_width(window: WebviewWindow, state: AppStateRef<'_>, width: u32) {
    state.popup_width.store(width.clamp(320, 1200), Ordering::Relaxed);
    state.ui_ready.store(true, Ordering::Relaxed);
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    // A popup sized by the user keeps its width; the default only matters again when that size is reset.
    if size_is_fixed(&state) || is_resizing(&state) {
        return;
    }
    // Copy the value out: holding the lock across window calls could deadlock against the main thread.
    let anchor = *state.anchor.lock().unwrap();
    if let Some(a) = anchor {
        place_popup(&window, &state, a, false);
    }
}

/// Pinned = stays open, can be dragged and resized. Unpinning snaps back to the tray corner.
#[tauri::command]
fn set_pinned(window: WebviewWindow, state: AppStateRef<'_>, pinned: bool) {
    let was = state.pinned.swap(pinned, Ordering::Relaxed);
    // Resizing works pinned or not; pinning only adds dragging and staying open.
    let _ = window.set_resizable(true);
    let _ = window.set_min_size(Some(LogicalSize::new(240.0, 320.0)));
    if pinned {
        // Only a spot the user can see is worth remembering: a hidden window (the UI restores the pinned
        // setting at start-up) reports a meaningless position, and the popup would later open there.
        if window.is_visible().unwrap_or(false) {
            if let Ok(p) = window.outer_position() {
                *state.pinned_pos.lock().unwrap() = Some(p);
            }
        }
    } else if was && window.is_visible().unwrap_or(false) {
        let anchor = *state.anchor.lock().unwrap();
        if let Some(a) = anchor {
            place_popup(&window, &state, a, size_is_fixed(&state));
        }
    }
}

/// Centre of a tray-icon rectangle in physical pixels; `None` when Windows reported an empty one
/// (it can for an icon that sits in the hidden-icons flyout).
fn rect_center(r: &Rect) -> Option<(f64, f64)> {
    let p = r.position.to_physical::<f64>(1.0);
    let s = r.size.to_physical::<f64>(1.0);
    (s.width >= 1.0 && s.height >= 1.0).then(|| (p.x + s.width / 2.0, p.y + s.height / 2.0))
}

/// Bottom-right corner of the primary monitor's work area: where the tray almost always is.
fn primary_corner(app: &AppHandle) -> Option<(f64, f64)> {
    let mon = app.primary_monitor().ok().flatten()?;
    let wa = mon.work_area();
    Some((
        wa.position.x as f64 + wa.size.width as f64 - 1.0,
        wa.position.y as f64 + wa.size.height as f64 - 1.0,
    ))
}

/// Place the popup next to the tray icon, clamped to the monitor work area (which already excludes the
/// taskbar, wherever it is docked). `anchor` is the icon centre when the user clicked it; otherwise
/// (hotkey, menu, second launch) the icon's current rectangle, the last known spot, the primary
/// monitor's bottom-right corner and finally the cursor are tried in that order.
fn position_popup(window: &WebviewWindow, state: &AppState, anchor: Option<(f64, f64)>, keep_size: bool) {
    let app = window.app_handle();
    let from_tray = || {
        app.tray_by_id("main")
            .and_then(|t| t.rect().ok().flatten())
            .and_then(|r| rect_center(&r))
    };
    let remembered = *state.anchor.lock().unwrap();
    let cursor = || window.cursor_position().ok().map(|p| (p.x, p.y));
    let Some(anchor) = anchor
        .or_else(from_tray)
        .or(remembered)
        .or_else(|| primary_corner(app))
        .or_else(cursor)
    else {
        return;
    };
    *state.anchor.lock().unwrap() = Some(anchor);
    place_popup(window, state, anchor, keep_size);
}

/// `keep_size` = leave the window size alone (a pinned popup keeps what the user chose).
fn place_popup(window: &WebviewWindow, state: &AppState, (ax, ay): (f64, f64), keep_size: bool) {
    let Ok(Some(mon)) = window.app_handle().monitor_from_point(ax, ay) else {
        return;
    };
    // Everything is computed with the *target* monitor's scale: the window may still sit on another one.
    let scale = mon.scale_factor();
    let margin = MARGIN * scale;
    let wa = mon.work_area();
    let (left, top) = (wa.position.x as f64, wa.position.y as f64);
    let (right, bottom) = (left + wa.size.width as f64, top + wa.size.height as f64);

    let (w, h) = if keep_size {
        match window.outer_size() {
            Ok(s) => (s.width as f64, s.height as f64),
            Err(_) => return,
        }
    } else {
        // Requested size, shrunk to fit small work areas (e.g. 1366x768 laptops).
        let want_w = state.popup_width.load(Ordering::Relaxed) as f64 * scale;
        let want_h = state.popup_height.load(Ordering::Relaxed) as f64 * scale;
        (
            want_w.min(wa.size.width as f64 - 2.0 * margin),
            want_h.min(wa.size.height as f64 - 2.0 * margin),
        )
    };
    let x = (ax - w / 2.0).clamp(left + margin, (right - w - margin).max(left + margin));
    let y = if ay > (top + bottom) / 2.0 { bottom - h - margin } else { top + margin }.max(top + margin);
    let pos = PhysicalPosition::new(x.round() as i32, y.round() as i32);

    // Move first (so the window is on the target monitor and any DPI change has been applied), then size it
    // in physical pixels, then settle the final position.
    let _ = window.set_position(pos);
    if !keep_size {
        let _ = window.set_size(PhysicalSize::new(w.round().max(1.0) as u32, h.round().max(1.0) as u32));
        let _ = window.set_position(pos);
    }
    *state.auto_pos.lock().unwrap() = Some(pos);
}

fn on_a_monitor(app: &AppHandle, p: &PhysicalPosition<i32>) -> bool {
    app.monitor_from_point(p.x as f64 + 24.0, p.y as f64 + 24.0)
        .ok()
        .flatten()
        .is_some()
}

/// While a window stays hidden its WebView2 runtime can give memory back: the view is marked invisible and the
/// low memory target is requested; showing the window undoes both first. A runtime too old to know the
/// setting simply ignores the request.
fn set_webview_trim(window: &WebviewWindow, trim: bool) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        };
        use windows::core::Interface;
        let _ = window.with_webview(move |wv| unsafe {
            let controller = wv.controller();
            let _ = controller.SetIsVisible(!trim);
            if let Ok(core) = controller.CoreWebView2() {
                if let Ok(core19) = core.cast::<ICoreWebView2_19>() {
                    let level = if trim {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
                    } else {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
                    };
                    let _ = core19.SetMemoryUsageTargetLevel(level);
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = (window, trim);
}

/// Takes the pages nobody is touching out of the working sets of this process and of the WebView2 processes it
/// started. They stay in memory as cached pages and come back on their first use, but while the popup is hidden
/// Task Manager shows what is really needed (a few MB) instead of what the popup left behind (100+ MB).
fn trim_working_sets() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA};
        let Some(list) = procs::list_now() else { return };
        for pid in procs::descendants(&list, std::process::id()) {
            if let Ok(h) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, false, pid) {
                let _ = EmptyWorkingSet(h);
                let _ = CloseHandle(h);
            }
        }
    }
}

fn trim_webview(app: &AppHandle, trim: bool) {
    let state = app.state::<Arc<AppState>>();
    if state.trimmed.swap(trim, Ordering::SeqCst) == trim {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        set_webview_trim(&w, trim);
    }
    if trim {
        // A moment later, once the page has let go of what it was drawing; not if the popup came back meanwhile.
        let st = state.inner().clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            if st.trimmed.load(Ordering::SeqCst) {
                trim_working_sets();
            }
        });
    }
}

/// Same for the right-click menu window, which is hidden almost all of the time.
fn trim_menu(app: &AppHandle, trim: bool) {
    let state = app.state::<Arc<AppState>>();
    if state.menu_trimmed.swap(trim, Ordering::SeqCst) == trim {
        return;
    }
    if let Some(w) = app.get_webview_window(MENU_LABEL) {
        set_webview_trim(&w, trim);
    }
}

fn show_popup(app: &AppHandle, anchor: Option<(f64, f64)>) {
    let Some(w) = app.get_webview_window("main") else { return };
    let state = app.state::<Arc<AppState>>();
    trim_webview(app, false);
    let pinned = state.pinned.load(Ordering::Relaxed);
    // A pinned popup reappears exactly where the user left it (if that spot still exists).
    let remembered = if pinned { *state.pinned_pos.lock().unwrap() } else { None };
    match remembered.filter(|p| on_a_monitor(app, p)) {
        Some(pos) => {
            let _ = w.set_position(pos);
        }
        None => position_popup(&w, &state, anchor, size_is_fixed(&state)),
    }
    // The page re-measures its tiles on every visit and the window follows (height, sometimes the column count): that
    // used to be visible as a quick resize right after the popup appeared. So the window is shown cloaked (invisible
    // but rendering) and revealed once the page reports `popup_settled`, or after `SETTLE_MAX`. A popup the user
    // sized (or pinned) never refits, and before the UI has loaded nobody could answer: those show straight away.
    let settle = cfg!(windows) && state.ui_ready.load(Ordering::Relaxed) && !size_is_fixed(&state);
    let token = now_ms().max(1);
    if settle {
        state.revealing.store(token, Ordering::SeqCst);
        set_cloak(&w, true);
    } else {
        state.revealing.store(0, Ordering::SeqCst);
        set_cloak(&w, false);
    }
    let _ = w.show();
    let _ = w.set_focus();
    // The UI starts every visit on the main view and re-measures its layout.
    let _ = app.emit("popup-shown", ());
    if settle {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SETTLE_MAX);
            finish_reveal(&app, token);
        });
    }
}

/// Cloak / uncloak a window with DWM: it stays shown and keeps rendering, but nothing of it is displayed.
fn set_cloak(w: &WebviewWindow, cloak: bool) {
    #[cfg(windows)]
    {
        #[link(name = "dwmapi")]
        unsafe extern "system" {
            fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const std::ffi::c_void, size: u32) -> i32;
        }
        const DWMWA_CLOAK: u32 = 13;
        if let Ok(h) = w.hwnd() {
            let value: i32 = cloak as i32;
            unsafe {
                DwmSetWindowAttribute(h.0 as isize, DWMWA_CLOAK, &value as *const i32 as *const std::ffi::c_void, 4);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (w, cloak);
}

/// Ends the pending reveal of the show that has `token` (a later show has its own token and is left alone).
fn finish_reveal(app: &AppHandle, token: u64) {
    let state = app.state::<Arc<AppState>>();
    if state.revealing.compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        set_cloak(&w, false);
        if w.is_visible().unwrap_or(false) {
            let _ = w.set_focus();
        }
    }
}

/// The page has measured its tiles after a show: the window already has its final size, show it.
#[tauri::command]
fn popup_settled(app: AppHandle, state: AppStateRef<'_>) {
    let token = state.revealing.load(Ordering::SeqCst);
    if token != 0 {
        finish_reveal(&app, token);
    }
}

fn toggle_popup(app: &AppHandle, anchor: Option<(f64, f64)>) {
    let Some(w) = app.get_webview_window("main") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    let state = app.state::<Arc<AppState>>();
    if now_ms().saturating_sub(state.last_blur_hide.load(Ordering::Relaxed)) < REOPEN_GUARD_MS {
        return;
    }
    show_popup(app, anchor);
}

// ------------------------------------------------------------------ right-click menu

/// How far the menu card keeps away from the screen edge / taskbar (logical pixels).
const MENU_GAP: f64 = 8.0;

fn menu_visible(app: &AppHandle) -> bool {
    app.get_webview_window(MENU_LABEL)
        .is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Top-left corner (physical px) of the menu window so that the card opens next to the pointer like a native tray
/// menu: above the pointer when it is in the lower half of the monitor, below it otherwise, and always inside the
/// work area (which excludes the taskbar). The window has a transparent frame of `pad` around the card; the card
/// itself keeps `gap` from the edge, so up to `pad - gap` of the frame may overhang.
fn menu_origin(
    (cx, cy): (f64, f64),
    (w, h): (f64, f64),
    pad: f64,
    gap: f64,
    (left, top, right, bottom): (f64, f64, f64, f64),
) -> (f64, f64) {
    let slack = (pad - gap).max(0.0);
    let x = (cx - pad).clamp(left - slack, (right - w + slack).max(left - slack));
    let y = if cy > (top + bottom) / 2.0 { cy - h + pad } else { cy - pad };
    (x, y.clamp(top - slack, (bottom - h + slack).max(top - slack)))
}

fn place_menu(window: &WebviewWindow, state: &AppState, (cx, cy): (f64, f64)) {
    let app = window.app_handle();
    let Some(mon) = app
        .monitor_from_point(cx, cy)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
    else {
        return;
    };
    let scale = mon.scale_factor();
    let w = (state.menu_width.load(Ordering::Relaxed) as f64 * scale).round().max(1.0);
    let h = (state.menu_height.load(Ordering::Relaxed) as f64 * scale).round().max(1.0);
    let wa = mon.work_area();
    let (left, top) = (wa.position.x as f64, wa.position.y as f64);
    let work = (left, top, left + wa.size.width as f64, top + wa.size.height as f64);
    let (x, y) = menu_origin((cx, cy), (w, h), MENU_PAD * scale, MENU_GAP * scale, work);
    let pos = PhysicalPosition::new(x.round() as i32, y.round() as i32);
    // Same order as the popup: move first (so the window is on the target monitor), size it in physical pixels, settle.
    let _ = window.set_position(pos);
    let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
    let _ = window.set_position(pos);
}

/// Right-click on the tray icon: the menu opens at the pointer.
fn show_menu(app: &AppHandle, cursor: (f64, f64)) {
    let Some(w) = app.get_webview_window(MENU_LABEL) else { return };
    let state = app.state::<Arc<AppState>>();
    trim_menu(app, false);
    *state.menu_cursor.lock().unwrap() = Some(cursor);
    place_menu(&w, &state, cursor);
    let _ = w.show();
    let _ = w.set_focus();
    // The page re-reads language, theme and the update offer every time it is shown.
    let _ = app.emit_to(MENU_LABEL, "menu-shown", ());
}

/// The menu page reports how big its card is (logical px, shadow frame included). The size changes when an update
/// entry appears; if that is reported while the menu is open (the offer was read after it was placed), it is placed again.
#[tauri::command]
fn set_menu_size(app: AppHandle, state: AppStateRef<'_>, width: u32, height: u32) {
    let (w, h) = (width.clamp(120, 600), height.clamp(60, 600));
    let (old_w, old_h) = (state.menu_width.swap(w, Ordering::Relaxed), state.menu_height.swap(h, Ordering::Relaxed));
    if (old_w, old_h) == (w, h) {
        return;
    }
    let cursor = *state.menu_cursor.lock().unwrap();
    if let (Some(cursor), Some(win)) = (cursor, app.get_webview_window(MENU_LABEL)) {
        if win.is_visible().unwrap_or(false) {
            place_menu(&win, &state, cursor);
        }
    }
}

/// What the menu page asks for every time it is shown.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MenuState {
    /// The panel is open and stays open (a pinned panel): the first entry then says "Hide" instead of "Show". An
    /// unpinned panel closes by itself the moment the menu takes the focus, so it never counts as open here.
    popup_open: bool,
}

#[tauri::command]
fn tray_menu_state(app: AppHandle, state: AppStateRef<'_>) -> MenuState {
    MenuState { popup_open: popup_visible(&app) && state.pinned.load(Ordering::Relaxed) }
}

/// The theme behind a "theme:<mode>" menu entry (anything else is not a theme choice).
fn theme_from_menu_id(id: &str) -> Option<&'static str> {
    match id {
        "theme:system" => Some("system"),
        "theme:dark" => Some("dark"),
        "theme:light" => Some("light"),
        _ => None,
    }
}

/// A menu entry was chosen: close the menu, then do it. Only the menu window may ask for this.
#[tauri::command]
fn tray_menu_action(app: AppHandle, window: WebviewWindow, id: String) {
    if window.label() != MENU_LABEL {
        return;
    }
    let _ = window.hide();
    match id.as_str() {
        "show" => show_popup(&app, None),
        // Only offered while a pinned panel is open.
        "hide" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
        }
        "settings" => {
            show_popup(&app, None);
            let _ = app.emit("open-settings", ());
        }
        // "Update · vX": the popup opens on the update screen (version, notes, the Update button). Nothing is
        // installed by the click itself.
        "update" => {
            show_popup(&app, None);
            let _ = app.emit("open-update", ());
        }
        "quit" => app.exit(0),
        // The Theme entry's choices: the main window owns the settings (like for the mini gauge): it records the theme,
        // which puts it on every window.
        other if theme_from_menu_id(other).is_some() => {
            let _ = app.emit_to("main", "theme-chosen", theme_from_menu_id(other));
        }
        "hud" => {
            let state = app.state::<Arc<AppState>>();
            let on = {
                let mut cfg = state.hud.lock().unwrap();
                cfg.on = !cfg.on;
                cfg.on
            };
            apply_hud(&app, &state);
            // The main window owns the settings: it records the new state.
            let _ = app.emit_to("main", "hud-toggled", on);
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ desktop mini gauge

fn hud_visible(app: &AppHandle) -> bool {
    app.get_webview_window(hud::LABEL).is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Settings from the main window (the only writer of the settings).
#[tauri::command]
fn set_hud(app: AppHandle, state: AppStateRef<'_>, config: hud::HudConfig) {
    *state.hud.lock().unwrap() = config.sanitized();
    apply_hud(&app, &state);
}

/// The gauge page asks for its settings when it loads.
#[tauri::command]
fn get_hud(state: AppStateRef<'_>) -> hud::HudConfig {
    state.hud.lock().unwrap().clone()
}

/// The gauge page measures its card and reports it (logical pixels).
#[tauri::command]
fn set_hud_size(window: WebviewWindow, width: f64, height: f64) {
    if window.label() != hud::LABEL || !(width > 0.0 && height > 0.0) {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let want = PhysicalSize::new((width.ceil() * scale).round() as u32, (height.ceil() * scale).round() as u32);
    if window.inner_size().ok() != Some(want) {
        let _ = window.set_size(want);
    }
}

/// Show / hide the gauge for the current settings and full-screen state; push the settings to its page.
fn apply_hud(app: &AppHandle, state: &AppState) {
    let cfg = state.hud.lock().unwrap().clone();
    let Some(w) = app.get_webview_window(hud::LABEL) else { return };
    let _ = app.emit_to(hud::LABEL, "hud-config", &cfg);
    let _ = w.set_ignore_cursor_events(cfg.locked);
    let want = cfg.on && !(cfg.hide_fullscreen && state.hud_fs_hidden.load(Ordering::Relaxed));
    let shown = w.is_visible().unwrap_or(false);
    if want && !shown {
        place_hud(app, state, &w);
        // Showing the gauge (its web view can take the keyboard focus) must not close the popup the switch was
        // clicked in: ignore the popup's blur for a moment and give it the focus back.
        let popup = app.get_webview_window("main").filter(|p| p.is_visible().unwrap_or(false));
        if popup.is_some() {
            state.blur_guard_until.store(now_ms() + HUD_FOCUS_GUARD_MS, Ordering::Relaxed);
        }
        let _ = w.show();
        if let Some(p) = popup {
            let _ = p.set_focus();
        }
    } else if !want && shown {
        let _ = w.hide();
    }
    // tao rewrites the extended style from its own flags on every change (click-through, show): put it back.
    make_tool_window(&w);
}

/// Where the user left it (if that spot is still on a monitor), else the top-right corner of the primary screen.
fn place_hud(app: &AppHandle, state: &AppState, w: &WebviewWindow) {
    let size = w.outer_size().map(|s| (s.width, s.height)).unwrap_or((200, 120));
    let areas: Vec<hud::Area> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let wa = m.work_area();
            (wa.position.x, wa.position.y, wa.size.width, wa.size.height)
        })
        .collect();
    let saved = *state.hud_pos.lock().unwrap();
    let pos = match saved {
        Some(p) if hud::on_screen(p, size, &areas) => p,
        _ => {
            let Ok(Some(mon)) = app.primary_monitor() else { return };
            let wa = mon.work_area();
            let margin = (16.0 * mon.scale_factor()).round() as i32;
            hud::default_pos((wa.position.x, wa.position.y, wa.size.width, wa.size.height), size, margin)
        }
    };
    let _ = w.set_position(PhysicalPosition::new(pos.x, pos.y));
}

/// Keeps the gauge out of Alt+Tab (a tool window). It never takes focus either: `"focusable": false` in
/// tauri.conf.json, which tao keeps as WS_EX_NOACTIVATE; a bit set here directly would be lost on its next restyle.
fn make_tool_window(w: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
    if let Ok(h) = w.hwnd() {
        let hwnd = HWND(h.0 as *mut std::ffi::c_void);
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if ex & WS_EX_TOOLWINDOW.0 as isize == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW.0 as isize);
            }
        }
    }
}

// ------------------------------------------------------------------ global hotkey

fn hotkey() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP)
}

/// Enable / disable the Ctrl+Alt+P shortcut. Returns whether it is registered afterwards
/// (false = another app already owns the combination).
#[tauri::command]
fn set_hotkey_enabled(app: AppHandle, enabled: bool) -> bool {
    let shortcuts = app.global_shortcut();
    let hk = hotkey();
    if enabled {
        if !shortcuts.is_registered(hk) {
            let _ = shortcuts.register(hk);
        }
    } else {
        let _ = shortcuts.unregister(hk);
    }
    shortcuts.is_registered(hk)
}

// ------------------------------------------------------------------ self-update
//
// Three separate things (updater.rs, updatestate.rs): the daily check only OFFERS an update (pill in the popup, entry
// in the tray menu, the update screen); the download starts when the user presses "Update" or, with "Install updates
// automatically" on, by itself in the background; the install replaces the exe and restarts the app quietly.

/// Installing replaces the running exe and restarts the app, so the automatic mode waits for a quiet moment:
/// popup closed and no latency test running.
fn can_install_now(app: &AppHandle, state: &AppState) -> bool {
    !popup_visible(app) && !state.ping.lock().unwrap().is_running()
}

/// The UI follows the update through events: `update-offer` (the offer, or null), `update-phase` and `update-progress`.
fn emit_update_phase(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let _ = app.emit("update-phase", state.update.phase());
}

fn emit_update_offer(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let _ = app.emit("update-offer", state.update.offer());
}

#[derive(Serialize, Clone)]
struct UpdateProgress {
    done: u64,
    /// 0 = the size is not known.
    total: u64,
}

fn install_and_exit(app: &AppHandle, pkg: &updater::Downloaded) -> Result<(), updater::UpdErr> {
    eprintln!("[updater] installing {}", pkg.version);
    // Once the exe is replaced, this exit must not count as proof that the NEW version is healthy.
    EXITING_FOR_UPDATE.store(true, Ordering::SeqCst);
    if let Err(e) = updater::install_and_relaunch(pkg, env!("CARGO_PKG_VERSION")) {
        EXITING_FOR_UPDATE.store(false, Ordering::SeqCst);
        return Err(e);
    }
    app.exit(0);
    Ok(())
}

/// An announcement that came with a server reply: validated, remembered and pushed to the UI.
fn show_notice(app: &AppHandle, state: &AppState, lang: &str, raw: &Option<serde_json::Value>) {
    let Some(raw) = raw else { return };
    let reply = serde_json::json!({ "notice": raw });
    if let Some(notice) = remote_notice::parse_notice(&reply, lang, updater::now_secs() as i64) {
        *state.notice.lock().unwrap() = Some(notice.clone());
        let _ = app.emit("notice", &notice);
    }
}

/// Does an offer download and install by itself? Only with "Install Updates Automatically" on AND "Check for Updates
/// Automatically" on (the install is the second half of the check; the boxes are greyed out that way in Settings), or
/// when the server calls the update mandatory. Never without a checksum to verify against: trusting the transport alone
/// is not enough for a silent install.
fn automatic_install(auto_check: bool, auto_update: bool, mandatory: bool, has_checksum: bool) -> bool {
    ((auto_check && auto_update) || mandatory) && has_checksum
}

/// The daily background check: due when "Check for Updates Automatically" is on and a day has passed since the last one.
fn daily_check_due(auto_check: bool, last: u64, now: u64) -> bool {
    auto_check && now.saturating_sub(last) >= 24 * 3600
}

/// The server's answer becomes the offer everybody sees (pill, tray menu, update screen). With the automatic mode on
/// (or a mandatory update) and a checksum to verify against, the download starts at once in the background and the
/// install follows at a quiet moment. Returns the offered version (None = up to date).
fn apply_offer(app: &AppHandle, state: &AppState, offer: Option<updater::Offer>) -> Option<String> {
    let version = offer.as_ref().map(|o| o.version.clone());
    // Silent / automatic installs never go without a checksum (see `automatic_install`).
    // A development build never replaces itself.
    let automatic = !cfg!(debug_assertions)
        && offer.as_ref().is_some_and(|o| {
            automatic_install(
                state.auto_check.load(Ordering::Relaxed),
                state.auto_update.load(Ordering::Relaxed),
                o.mandatory,
                o.sha256.is_some(),
            )
        });
    if state.update.set_offer(offer) {
        emit_update_offer(app);
    }
    emit_update_phase(app); // a new offer can reset an old error
    if automatic && state.update.start(false) == updatestate::Start::Spawn {
        emit_update_phase(app);
        spawn_update_job(app.clone());
    }
    version
}

/// One check at a time: a manual "Check" during the daily check would otherwise ask twice at once.
static UPDATE_CHECK: Mutex<()> = Mutex::new(());

/// Asks the server (nothing is downloaded by this). Returns the offered version, None = up to date.
fn run_update_check(app: &AppHandle) -> Result<Option<String>, String> {
    let _one_at_a_time = UPDATE_CHECK.lock().unwrap();
    let state = app.state::<Arc<AppState>>();
    let lang = state.lang.lock().unwrap().clone();
    let checked = updater::check(env!("CARGO_PKG_VERSION"))?;
    show_notice(app, &state, &lang, &checked.notice);
    Ok(apply_offer(app, &state, checked.offer))
}

/// The download of the offer (the part that needs the network and the disk).
fn fetch_update(
    app: &AppHandle,
    state: &AppState,
    offer: &updater::Offer,
) -> Result<updater::Downloaded, updater::UpdErr> {
    // Better to learn that the folder can't take the new exe before anything is downloaded.
    if let Ok(exe) = std::env::current_exe() {
        updater::check_writable(&exe)?;
    }
    let cancel = state.update.cancel_flag();
    let mut throttle = updater::Throttle::new(Duration::from_millis(200));
    updater::download(offer, &cancel, |done, total| {
        // ~5 reports a second; the last one (100%) always goes out.
        if (total > 0 && done >= total) || throttle.due(Instant::now()) {
            let _ = app.emit("update-progress", UpdateProgress { done, total });
        }
    })
}

fn spawn_update_job(app: AppHandle) {
    std::thread::spawn(move || run_update_job(&app));
}

/// The worker behind "Update" and the automatic mode: download if there is no package for the offered version yet, then
/// install at the right moment (at once for the user's press, at a quiet moment for the automatic mode). Only one
/// runs at a time (`UpdateState::start` decides when to spawn it); the state machine says what to do next.
fn run_update_job(app: &AppHandle) {
    use updatestate::{Outcome, Step};
    let state = app.state::<Arc<AppState>>().inner().clone();
    loop {
        match state.update.next_step(can_install_now(app, &state)) {
            Step::Stop => {
                state.update.end_job();
                emit_update_phase(app);
                return;
            }
            Step::Wait => std::thread::sleep(Duration::from_millis(300)),
            Step::Download => {
                let Some(offer) = state.update.begin_download() else { continue };
                emit_update_phase(app);
                let result = fetch_update(app, &state, &offer);
                match &result {
                    Ok(_) | Err(updater::UpdErr::Cancelled) => {}
                    Err(e) => eprintln!("[updater] {e}"),
                }
                let outcome = state.update.finish_download(result);
                emit_update_phase(app);
                if matches!(outcome, Outcome::Cancelled | Outcome::Failed) {
                    return;
                }
            }
            Step::Install => {
                let Some(pkg) = state.update.begin_install() else { continue };
                emit_update_phase(app);
                match install_and_exit(app, &pkg) {
                    Ok(()) => return, // the app is exiting; never start a second install
                    Err(e) => {
                        eprintln!("[updater] {e}");
                        // The old version keeps running; the update screen shows the error and "Retry".
                        state.update.install_failed(e.ui());
                        emit_update_phase(app);
                        return;
                    }
                }
            }
        }
    }
}

/// The update on offer (None = up to date, or nothing asked yet): the popup, Settings and the tray menu read it.
#[tauri::command]
fn get_update_offer(state: AppStateRef<'_>) -> Option<updater::Offer> {
    state.update.offer()
}

/// idle / downloading / ready / installing / failed (+ error).
#[tauri::command]
fn get_update_phase(state: AppStateRef<'_>) -> updatestate::Phase {
    state.update.phase()
}

/// The update screen's "Update" button: download (or join the download that is already running), then install and
/// restart the app. Progress and results come as events.
#[tauri::command]
fn start_update(app: AppHandle, state: AppStateRef<'_>) -> Result<(), String> {
    match state.update.start(true) {
        updatestate::Start::NoOffer => return Err("no_offer".into()),
        updatestate::Start::Spawn => {
            emit_update_phase(&app);
            spawn_update_job(app.clone());
        }
        updatestate::Start::Joined => emit_update_phase(&app),
        updatestate::Start::Installing => {}
    }
    Ok(())
}

/// "Cancel" during the download.
#[tauri::command]
fn cancel_update(app: AppHandle, state: AppStateRef<'_>) -> bool {
    let cancelled = state.update.cancel();
    if cancelled {
        emit_update_phase(&app);
    }
    cancelled
}

/// Right after an update the new version tells the user so (the app itself starts hidden in the tray): a Windows
/// notification. The UI hands over the text in its language. Works once and only for a start that came from an
/// update (`--updated <old version>`); returns whether it was shown.
#[tauri::command]
async fn announce_update(app: AppHandle, title: String, body: String) -> bool {
    let clip = |s: String, n: usize| s.trim().chars().take(n).collect::<String>();
    let (title, body) = (clip(title, 100), clip(body, 250));
    if title.is_empty() || body.is_empty() {
        return false;
    }
    let state = app.state::<Arc<AppState>>().inner().clone();
    if state.updated_from.lock().unwrap().take().is_none() {
        return false;
    }
    let data_dir = app.path().app_data_dir().ok();
    let h = app.clone();
    let shown = tauri::async_runtime::spawn_blocking(move || {
        toast::Toaster::new(&toast::app_id(&h), data_dir).show(&title, &body, {
            let h = h.clone();
            move || open_from_toast(&h)
        })
    })
    .await;
    match shown {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            eprintln!("[updater] notification: {e}");
            false
        }
        Err(_) => false,
    }
}

/// Manual "check now" from Settings. Returns "up_to_date", "offer:<version>" or "dev".
#[tauri::command]
async fn check_update_now(app: AppHandle) -> Result<String, String> {
    if cfg!(debug_assertions) {
        return Ok("dev".into());
    }
    let offered = tauri::async_runtime::spawn_blocking(move || run_update_check(&app))
        .await
        .map_err(|e| e.to_string())??;
    Ok(match offered {
        Some(v) => format!("offer:{v}"),
        None => "up_to_date".into(),
    })
}

/// Once a day (checked hourly, so a laptop that was asleep catches up), silently, unless "Check for Updates
/// Automatically" is off. This only asks: what happens to an offer is up to the automatic mode (the setting) or the user.
fn spawn_update_loop(app: AppHandle, state: Arc<AppState>, dir: Option<PathBuf>) {
    if cfg!(debug_assertions) {
        return;
    }
    let stamp = dir.map(|d| d.join("update_check.txt"));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(45));
        loop {
            let last = stamp
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|t| t.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let now = updater::now_secs();
            if daily_check_due(state.auto_check.load(Ordering::Relaxed), last, now) {
                // Success records "now"; a failure retries in ~6 h instead of hammering the server.
                let record = match run_update_check(&app) {
                    Ok(_) => now,
                    Err(e) => {
                        eprintln!("[updater] {e}");
                        now.saturating_sub(18 * 3600)
                    }
                };
                if let Some(p) = &stamp {
                    let _ = std::fs::write(p, record.to_string());
                }
            }
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

/// After an update the new exe is started with `--wait-pid <old pid>`: let the old process finish
/// exiting first, otherwise the single-instance guard would send our launch to it and quit.
fn wait_for_previous_instance() {
    let args: Vec<String> = std::env::args().collect();
    let Some(pid) = updater::arg_value(&args, "--wait-pid").and_then(|p| p.parse::<u32>().ok()) else {
        return;
    };
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        // If the process is already gone, OpenProcess fails and there is nothing to wait for.
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(h, 15_000);
            let _ = CloseHandle(h);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}

// ------------------------------------------------------------------ tray tooltip

fn fmt_rate(b: f64) -> String {
    const U: [&str; 4] = ["B/s", "KB/s", "MB/s", "GB/s"];
    let (mut v, mut i) = (b, 0);
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{:.0} {}", v, U[i]) } else { format!("{:.1} {}", v, U[i]) }
}

fn tooltip(m: &Metrics) -> String {
    let mem = if m.mem_total > 0 { m.mem_used as f64 / m.mem_total as f64 * 100.0 } else { 0.0 };
    let mut s = format!(
        "PulseTray\nCPU {:.0}%  RAM {:.0}%\n↓ {}  ↑ {}",
        m.cpu_usage,
        mem,
        fmt_rate(m.net_down),
        fmt_rate(m.net_up)
    );
    if m.battery.present {
        s.push_str(&format!("\nBattery {}%", m.battery.percent));
    }
    s
}

/// The number the tray icon shows in `mode` (None: the logo, or no value yet, e.g. no GPU).
fn tray_value(mode: trayicon::Mode, m: &Metrics) -> Option<u32> {
    let v = match mode {
        trayicon::Mode::Logo => return None,
        trayicon::Mode::Cpu => (m.cpu_cores > 0).then_some(m.cpu_usage as f64)?,
        trayicon::Mode::Ram => (m.mem_total > 0).then(|| m.mem_used as f64 / m.mem_total as f64 * 100.0)?,
        // The GPU with the most dedicated memory, like the GPU tile (the dGPU over the iGPU).
        trayicon::Mode::Gpu => m.gpus.iter().max_by_key(|g| g.vram_total).map(|g| g.usage as f64)?,
    };
    Some(v.round().clamp(0.0, 100.0) as u32)
}

// ------------------------------------------------------------------ app

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    STARTED.get_or_init(Instant::now);
    wait_for_previous_instance();
    // The first starts after an update are on probation: a new version that keeps dying is rolled back.
    if updater::startup_guard() {
        return;
    }

    let context = tauri::generate_context!();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_popup(app, None);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if shortcut == &hotkey() && event.state() == ShortcutState::Pressed {
                        toggle_popup(app, None);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_metrics,
            get_history,
            get_hardware,
            top_apps_at,
            battery_history,
            set_alerts,
            alert_test,
            set_popup_width,
            set_popup_height,
            popup_settled,
            set_custom_size,
            start_resize,
            start_move,
            set_features,
            set_ping_targets,
            ping_start,
            ping_stop,
            set_ping_continuous,
            get_ping_state,
            resize_popup,
            set_auto_update,
            set_auto_check,
            get_notice,
            open_site,
            set_pinned,
            set_hotkey_enabled,
            set_language,
            autostart_get,
            autostart_set,
            check_update_now,
            get_update_offer,
            get_update_phase,
            start_update,
            cancel_update,
            announce_update,
            hide_popup,
            quit_app,
            set_menu_size,
            tray_menu_action,
            tray_menu_state,
            app_infos,
            reveal_app,
            end_app,
            set_tray_icon,
            set_hud,
            get_hud,
            set_hud_size
        ])
        .setup(|app| {
            let state = Arc::new(AppState {
                latest: Mutex::new(Metrics::default()),
                hardware: Mutex::new(None),
                last_blur_hide: AtomicU64::new(0),
                revealing: AtomicU64::new(0),
                blur_guard_until: AtomicU64::new(0),
                popup_width: AtomicU32::new(DEFAULT_W),
                popup_height: AtomicU32::new(DEFAULT_H),
                auto_update: AtomicBool::new(true),
                auto_check: AtomicBool::new(true),
                lang: Mutex::new("en".into()),
                notice: Mutex::new(None),
                update: updatestate::UpdateState::new(),
                // `--updated <old version>`: this start comes from an update (the notification is still to be shown).
                updated_from: Mutex::new(
                    updater::updated_from(&std::env::args().collect::<Vec<_>>())
                        .filter(|v| v != env!("CARGO_PKG_VERSION")),
                ),
                pinned: AtomicBool::new(false),
                custom_size: AtomicBool::new(false),
                refit_height: AtomicBool::new(false),
                resizing_since: AtomicU64::new(0),
                pinned_pos: Mutex::new(None),
                auto_pos: Mutex::new(None),
                anchor: Mutex::new(None),
                trimmed: AtomicBool::new(false),
                features: Mutex::new(Features::default()),
                ping_targets: Mutex::new(vec![ping::GATEWAY.into(), "1.1.1.1".into(), "8.8.8.8".into()]),
                ping: Mutex::new(PingState::default()),
                ui_ready: AtomicBool::new(false),
                menu_width: AtomicU32::new(MENU_DEFAULT_W),
                menu_height: AtomicU32::new(MENU_DEFAULT_H),
                menu_cursor: Mutex::new(None),
                menu_trimmed: AtomicBool::new(false),
                history: Mutex::new(History::default()),
                tray_mode: AtomicU8::new(trayicon::Mode::Logo.as_u8()),
                hud: Mutex::new(hud::HudConfig::default()),
                hud_fs_hidden: AtomicBool::new(false),
                hud_pos: Mutex::new(hud::load_pos(app.path().app_data_dir().ok().as_deref())),
                hud_pos_dirty: AtomicBool::new(false),
                alerts: Mutex::new(alerts::AlertConfig::default()),
                battery_report: Mutex::new(None),
                // one session name per app identifier: a test build must not stop the installed copy's
                fps: Mutex::new(fps::FpsMeter::new(format!("PulseTray FPS ({})", app.config().identifier))),
            });
            app.manage(state.clone());

            // The windows are created only now (`"create": false` in tauri.conf.json): a page that loads before the
            // state above is managed would have its start-up commands (popup width, pinned, features...) rejected.
            for cfg in app.config().app.windows.clone() {
                tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?.build()?;
            }

            // Ctrl+Alt+P. If another app owns it the registration just fails; Settings shows that.
            let _ = app.global_shortcut().register(hotkey());

            // Popup window events.
            let window = app.get_webview_window("main").expect("main window");
            {
                let w = window.clone();
                let st = state.clone();
                window.on_window_event(move |e| match e {
                    // Hide when it loses focus (unless pinned).
                    WindowEvent::Focused(false) => {
                        if st.pinned.load(Ordering::Relaxed) || is_resizing(&st) {
                            return;
                        }
                        // The desktop gauge was just shown from the popup's own Settings: that moves the focus, the
                        // user did not click elsewhere.
                        if now_ms() < st.blur_guard_until.load(Ordering::Relaxed) {
                            return;
                        }
                        if w.is_visible().unwrap_or(false) {
                            st.last_blur_hide.store(now_ms(), Ordering::Relaxed);
                            let _ = w.hide();
                        }
                    }
                    // Alt+F4 / a close request must only hide the popup: it is the app's only window,
                    // and closing it would quit the app together with its tray icon.
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                    // Remember where a pinned popup was dragged to.
                    WindowEvent::Moved(pos) => {
                        if st.pinned.load(Ordering::Relaxed) && w.is_visible().unwrap_or(false) {
                            *st.pinned_pos.lock().unwrap() = Some(*pos);
                        }
                    }
                    _ => {}
                });
            }

            // Desktop mini gauge: a tool window that never takes focus; remembers where it was dragged to.
            if let Some(hud_window) = app.get_webview_window(hud::LABEL) {
                make_tool_window(&hud_window);
                let st = state.clone();
                let app_handle = app.handle().clone();
                let w = hud_window.clone();
                hud_window.on_window_event(move |e| match e {
                    WindowEvent::Moved(pos) => {
                        if w.is_visible().unwrap_or(false) {
                            *st.hud_pos.lock().unwrap() = Some(hud::Pos { x: pos.x, y: pos.y });
                            st.hud_pos_dirty.store(true, Ordering::Relaxed);
                        }
                    }
                    // Alt+F4 on the gauge switches it off (the main window records that).
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        st.hud.lock().unwrap().on = false;
                        let _ = w.hide();
                        let _ = app_handle.emit_to("main", "hud-toggled", false);
                    }
                    _ => {}
                });
            }
            // The main page sends the gauge settings as soon as it loads, and it can load while the later windows
            // are still being built (creating a web view pumps messages, so its `set_hud` runs in between): the
            // settings were stored but there was no gauge window to show yet, and it only appeared after a switch
            // off/on. Apply them now that every window exists.
            apply_hud(app.handle(), &state);

            // Right-click menu window: closes when it loses focus, like any menu.
            let menu_window = app.get_webview_window(MENU_LABEL).expect("menu window");
            {
                let w = menu_window.clone();
                menu_window.on_window_event(move |e| match e {
                    WindowEvent::Focused(false) => {
                        let _ = w.hide();
                    }
                    WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                    _ => {}
                });
            }

            // Tray icon: left click = the popup, right click = the menu window.
            let tray = TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("app icon").clone())
                .tooltip("PulseTray")
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, rect, position, .. } = event {
                        match button {
                            MouseButton::Left => {
                                // An empty icon rectangle (hidden-icons flyout): the click position is the next best anchor.
                                let anchor = rect_center(&rect).unwrap_or((position.x, position.y));
                                toggle_popup(tray.app_handle(), Some(anchor));
                            }
                            MouseButton::Right => show_menu(tray.app_handle(), (position.x, position.y)),
                            _ => {}
                        }
                    }
                })
                .build(app)?;

            let handle = app.handle().clone();
            let logo = app.default_window_icon().map(|i| i.clone().to_owned());

            // A manual launch shows the panel once so a first-time user sees something happen; an
            // autostart launch (--minimized) and an update restart stay quietly in the tray.
            if !std::env::args().any(|a| a == "--minimized") {
                let handle = handle.clone();
                let state = state.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(900));
                    // Wait (at most 5 s) until the UI has reported its width, so the popup opens at the right size.
                    let waited = Instant::now();
                    while !state.ui_ready.load(Ordering::Relaxed) && waited.elapsed() < Duration::from_secs(5) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    show_popup(&handle, None);
                });
            }

            // Static hardware facts (WMI is slow, so off the sampler thread).
            {
                let handle = handle.clone();
                let state = state.clone();
                std::thread::spawn(move || {
                    let hw = hardware::load();
                    *state.hardware.lock().unwrap() = Some(hw.clone());
                    let _ = handle.emit("hardware", &hw);
                });
            }

            let data_dir = app.path().app_data_dir().ok();
            // A package an earlier run left in %TEMP% (quit while it was waiting or downloading) goes now.
            std::thread::spawn(updater::remove_stale_download);
            // What the last update left behind (the previous version) is kept until this one has run for a minute.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(60));
                updater::confirm_healthy();
            });
            spawn_update_loop(handle.clone(), state.clone(), data_dir.clone());
            spawn_ping_thread(handle.clone(), state.clone());

            // Sampler loop: every second while the popup is visible, every 2 s otherwise (the charts' history
            // keeps going in the tray; the expensive parts, like the process list, only run on screen).
            let hud_dir = data_dir.clone();
            let alert_dir = data_dir.clone();
            std::thread::spawn(move || {
                let mut sampler = Sampler::new(data_dir);
                let mut alert_engine = alerts::Engine::new(alert_dir.clone());
                let mut toaster = toast::Toaster::new(&toast::app_id(&handle), alert_dir);
                // Alerts that came up while a game / full-screen video was in front: sent when it is gone.
                let mut held: Vec<alerts::Alert> = Vec::new();
                // `Instant` counts from boot on Windows: right after a fast start "now minus a minute" does not exist.
                let long_ago = Instant::now().checked_sub(Duration::from_secs(60)).unwrap_or_else(Instant::now);
                let (mut last_tip, mut last_tip_text) = (long_ago, String::new());
                // When the popup was last seen hidden, to let the web view give memory back after a quiet spell.
                let mut hidden_since: Option<Instant> = None;
                let mut menu_hidden_since: Option<Instant> = None;
                // What the tray icon shows now: mode and number (None = the logo).
                let mut drawn: (trayicon::Mode, Option<u32>) = (trayicon::Mode::Logo, None);
                let mut loops: u64 = 0;
                std::thread::sleep(Duration::from_millis(500));
                loop {
                    loops += 1;
                    let visible = popup_visible(&handle);
                    // Mini gauge: hide it while a full-screen app (game, video, presentation) is in front.
                    let hud_cfg = state.hud.lock().unwrap().clone();
                    if hud_cfg.on && hud_cfg.hide_fullscreen {
                        if loops % 2 == 0 {
                            let fs = fullscreen::is_fullscreen_app();
                            if state.hud_fs_hidden.swap(fs, Ordering::Relaxed) != fs {
                                apply_hud(&handle, &state);
                            }
                        }
                    } else if state.hud_fs_hidden.swap(false, Ordering::Relaxed) {
                        apply_hud(&handle, &state);
                    }
                    // Safety net: switched on, no full-screen app in front, yet not on screen — show it.
                    if hud_cfg.on && !state.hud_fs_hidden.load(Ordering::Relaxed) && !hud_visible(&handle) {
                        apply_hud(&handle, &state);
                    }
                    let hud_shown = hud_cfg.on && hud_visible(&handle);
                    if state.hud_pos_dirty.swap(false, Ordering::Relaxed) {
                        if let Some(pos) = *state.hud_pos.lock().unwrap() {
                            hud::save_pos(hud_dir.as_ref(), pos);
                        }
                    }
                    if visible {
                        hidden_since = None;
                        // Normally show_popup has done this already; this also repairs the (tiny) race with a trim.
                        trim_webview(&handle, false);
                    } else if hidden_since.get_or_insert_with(Instant::now).elapsed() >= TRIM_AFTER && !popup_visible(&handle) {
                        trim_webview(&handle, true);
                    }
                    if menu_visible(&handle) {
                        menu_hidden_since = None;
                        trim_menu(&handle, false);
                    } else if menu_hidden_since.get_or_insert_with(Instant::now).elapsed() >= TRIM_AFTER
                        && !menu_visible(&handle)
                    {
                        trim_menu(&handle, true);
                    }
                    let mut features = *state.features.lock().unwrap();
                    let tray_mode = trayicon::Mode::from_u8(state.tray_mode.load(Ordering::Relaxed));
                    // A GPU number in the tray (or on the mini gauge) needs the GPU counters even when the GPU tile is off.
                    features.gpu |= tray_mode == trayicon::Mode::Gpu || (hud_shown && hud_cfg.shows("gpu"));
                    let alert_cfg = state.alerts.lock().unwrap().clone();
                    features.alerts = alert_cfg.any();
                    // The temperature alert needs the GPU sensors in the background too.
                    features.gpu |= alert_cfg.temp;
                    let mut m = sampler.sample(visible, features);
                    // FPS on the mini gauge: the trace session runs only while the gauge shows it.
                    {
                        let want_fps = hud_shown && hud_cfg.shows("fps");
                        let mut meter = state.fps.lock().unwrap();
                        meter.set_running(want_fps);
                        if want_fps {
                            m.fps = Some(meter.read());
                        }
                    }
                    if alert_cfg.any() {
                        let drives = state.hardware.lock().unwrap().as_ref().map(|h| h.drives.clone()).unwrap_or_default();
                        held.extend(alert_engine.check(&alert_cfg, &m, &drives, Instant::now()));
                        // Not over a game or a full-screen video: kept until it is gone (Windows would bury them anyway).
                        if !held.is_empty() && !fullscreen::is_fullscreen_app() {
                            for a in held.drain(..) {
                                let h = handle.clone();
                                let _ = toaster.show(&a.title, &a.body, move || open_from_toast(&h));
                            }
                        }
                    } else {
                        held.clear();
                    }
                    let want = (tray_mode, tray_value(tray_mode, &m));
                    if want != drawn {
                        let icon = match want {
                            (mode, Some(v)) if mode != trayicon::Mode::Logo => {
                                let size = trayicon::icon_size();
                                trayicon::render(mode, v, size).map(|px| tauri::image::Image::new_owned(px, size, size))
                            }
                            _ => logo.clone(),
                        };
                        if let Some(icon) = icon {
                            let _ = tray.set_icon(Some(icon));
                        }
                        drawn = want;
                    }
                    // The tooltip only needs to be roughly current: refresh it every ~5 s, and only on change.
                    if last_tip.elapsed() >= Duration::from_secs(5) {
                        let tip = tooltip(&m);
                        if tip != last_tip_text {
                            let _ = tray.set_tooltip(Some(tip.clone()));
                            last_tip_text = tip;
                        }
                        last_tip = Instant::now();
                    }
                    state.history.lock().unwrap().push(&m);
                    if visible {
                        let _ = handle.emit("metrics", &m);
                    } else if hud_shown {
                        // Only the gauge: the hidden popup page doesn't need to redraw its charts.
                        let _ = handle.emit_to(hud::LABEL, "metrics", &m);
                    }
                    *state.latest.lock().unwrap() = m;

                    // Slower tick while hidden, but wake as soon as the popup opens so it fills in quickly.
                    // The mini gauge keeps the 1 s pace (without the popup's costly extras like the process list).
                    let interval = Duration::from_millis(if visible || hud_shown { 1000 } else { 2000 });
                    let start = Instant::now();
                    while start.elapsed() < interval {
                        std::thread::sleep(Duration::from_millis(150));
                        if popup_visible(&handle) != visible {
                            break;
                        }
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .expect("error while running PulseTray");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            // An ETW session outlives the process that started it: stop the FPS one first.
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                state.fps.lock().unwrap().close();
            }
            // A clean exit after a while proves this version works: what the last update left behind can go.
            // (Not the exit that installs the next update: by then the exe on disk is already the new one.)
            let up = STARTED.get().map(|t| t.elapsed()).unwrap_or_default();
            if updater::healthy_on_exit(up, EXITING_FOR_UPDATE.load(Ordering::SeqCst)) {
                updater::confirm_healthy();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{Position, Size};

    fn rect(x: f64, y: f64, w: u32, h: u32) -> Rect {
        Rect { position: Position::Physical(PhysicalPosition::new(x as i32, y as i32)), size: Size::Physical(PhysicalSize::new(w, h)) }
    }

    #[test]
    fn rect_center_is_the_middle_of_a_real_icon_rect() {
        assert_eq!(rect_center(&rect(3000.0, 1400.0, 24, 40)), Some((3012.0, 1420.0)));
    }

    #[test]
    fn an_empty_rect_is_not_an_anchor() {
        assert_eq!(rect_center(&rect(0.0, 0.0, 0, 0)), None);
        assert_eq!(rect_center(&rect(500.0, 500.0, 0, 30)), None);
    }

    const AREA: (i32, i32, i32, i32) = (0, 0, 1920, 1040);

    #[test]
    fn a_popup_at_the_bottom_right_corner_keeps_its_corner_when_it_shrinks() {
        // Advanced 736x980 against the bottom right (8 px margin) becomes classic 564x700.
        let pos = (1920 - 736 - 8, 1040 - 980 - 8);
        assert_eq!(keep_edges(AREA, pos, (736, 980), (564, 700), 24, true), (1920 - 564 - 8, 1040 - 700 - 8));
        // ... and back: it grows to the top left, not off the screen.
        let pos = (1920 - 564 - 8, 1040 - 700 - 8);
        assert_eq!(keep_edges(AREA, pos, (564, 700), (736, 980), 24, true), (1920 - 736 - 8, 1040 - 980 - 8));
    }

    #[test]
    fn a_popup_in_the_middle_keeps_its_top_left_corner() {
        assert_eq!(keep_edges(AREA, (400, 100), (736, 800), (564, 600), 24, true), (400, 100));
    }

    #[test]
    fn only_the_edge_that_was_close_is_kept() {
        // Against the bottom only: x stays, the bottom edge stays.
        assert_eq!(keep_edges(AREA, (400, 1040 - 800 - 8), (736, 800), (564, 600), 24, true), (400, 1040 - 600 - 8));
        // Against the right only: y stays.
        assert_eq!(keep_edges(AREA, (1920 - 736 - 8, 100), (736, 800), (564, 600), 24, true), (1920 - 564 - 8, 100));
    }

    #[test]
    fn a_popup_at_the_top_of_the_screen_stays_at_the_top() {
        // The taskbar is at the top: the popup hangs below it and keeps its top edge.
        assert_eq!(keep_edges(AREA, (1920 - 736 - 8, 8), (736, 800), (564, 600), 24, false), (1920 - 564 - 8, 8));
        // As tall as the screen: both edges are close, the tray decides.
        assert_eq!(keep_edges(AREA, (100, 8), (500, 1024), (500, 600), 24, false), (100, 8));
        assert_eq!(keep_edges(AREA, (100, 8), (500, 1024), (500, 600), 24, true), (100, 1040 - 600 - 8));
    }

    #[test]
    fn a_resized_popup_never_leaves_the_work_area() {
        // Grows in the middle of the screen: pulled back in at the bottom / right.
        assert_eq!(keep_edges(AREA, (1400, 600), (400, 300), (736, 980), 24, true), (1920 - 736, 1040 - 980));
        // Bigger than the work area: starts at its top left.
        assert_eq!(keep_edges(AREA, (300, 300), (400, 300), (3000, 2000), 24, true), (0, 0));
        // A second monitor to the left of the primary one (negative coordinates).
        let left = (-1920, 0, 0, 1040);
        assert_eq!(keep_edges(left, (-736 - 8, 100), (736, 700), (564, 500), 24, true), (-564 - 8, 100));
    }

    const WORK: (f64, f64, f64, f64) = (0.0, 0.0, 1920.0, 1040.0); // 1080p with a 40 px taskbar at the bottom

    #[test]
    fn menu_opens_above_the_pointer_when_the_taskbar_is_at_the_bottom() {
        // Pointer on the taskbar: the card sits right above it.
        let (x, y) = menu_origin((800.0, 1060.0), (196.0, 156.0), 12.0, 8.0, WORK);
        assert_eq!(x, 788.0);
        assert_eq!(y, 1040.0 - 156.0 + 4.0); // card bottom = work-area bottom - gap
    }

    #[test]
    fn menu_stays_on_screen_near_the_right_edge() {
        let (x, _) = menu_origin((1900.0, 1060.0), (196.0, 156.0), 12.0, 8.0, WORK);
        assert_eq!(x, 1920.0 - 196.0 + 4.0); // card right edge = screen edge - gap
    }

    #[test]
    fn menu_opens_below_the_pointer_when_the_taskbar_is_at_the_top() {
        let work = (0.0, 40.0, 1920.0, 1080.0);
        let (_, y) = menu_origin((1000.0, 20.0), (196.0, 156.0), 12.0, 8.0, work);
        assert_eq!(y, 40.0 - 4.0); // card top = work-area top + gap
    }

    #[test]
    fn menu_follows_the_pointer_when_there_is_room() {
        let (x, y) = menu_origin((600.0, 300.0), (196.0, 156.0), 12.0, 8.0, WORK);
        assert_eq!((x, y), (588.0, 288.0)); // the card's top-left corner is at the pointer
    }

    #[test]
    fn menu_larger_than_the_work_area_does_not_panic() {
        let (x, y) = menu_origin((10.0, 10.0), (4000.0, 3000.0), 12.0, 8.0, WORK);
        assert_eq!((x, y), (-4.0, -4.0));
    }

    #[test]
    fn only_the_three_themes_are_menu_choices() {
        assert_eq!(theme_from_menu_id("theme:system"), Some("system"));
        assert_eq!(theme_from_menu_id("theme:dark"), Some("dark"));
        assert_eq!(theme_from_menu_id("theme:light"), Some("light"));
        for other in ["theme", "theme:", "theme:blue", "show", "hide", "quit", "language:tr", ""] {
            assert_eq!(theme_from_menu_id(other), None, "{other}");
        }
    }

    #[test]
    fn the_daily_check_follows_the_automatic_check_setting() {
        const DAY: u64 = 24 * 3600;
        assert!(daily_check_due(true, 0, 5 * DAY), "never checked: due");
        assert!(daily_check_due(true, 1000, 1000 + DAY), "a full day later");
        assert!(!daily_check_due(true, 1000, 1000 + DAY - 1), "not yet");
        assert!(!daily_check_due(true, 5000, 1000), "a clock set back never makes it due");
        assert!(!daily_check_due(false, 0, 5 * DAY), "switched off: never, however long ago");
    }

    #[test]
    fn the_automatic_install_needs_the_check_and_a_checksum() {
        // (auto_check, auto_update, mandatory, has_checksum)
        assert!(automatic_install(true, true, false, true), "both boxes on");
        assert!(!automatic_install(true, false, false, true), "install box off");
        assert!(!automatic_install(false, true, false, true), "check box off: the install waits for it");
        assert!(!automatic_install(true, true, false, false), "no checksum: only offered");
        assert!(automatic_install(false, false, true, true), "a mandatory update goes on its own");
        assert!(!automatic_install(false, false, true, false), "mandatory but unverifiable: still only offered");
    }

    #[test]
    fn tooltip_formats_rates_and_battery() {
        assert_eq!(fmt_rate(512.0), "512 B/s");
        assert_eq!(fmt_rate(1536.0), "1.5 KB/s");
        assert_eq!(fmt_rate(5.0 * 1024.0 * 1024.0), "5.0 MB/s");
        let m = Metrics { cpu_usage: 12.4, mem_used: 5, mem_total: 10, ..Default::default() };
        let t = tooltip(&m);
        assert!(t.contains("CPU 12%") && t.contains("RAM 50%") && !t.contains("Battery"), "{t}");
    }

    #[test]
    fn instant_subtraction_near_boot_does_not_panic() {
        // What the sampler thread does at start-up; on a machine up for under a minute the naive
        // `Instant::now() - 60 s` would panic (and release builds abort on panic).
        let _ = Instant::now().checked_sub(Duration::from_secs(60 * 60 * 24 * 365 * 100)).unwrap_or_else(Instant::now);
    }
}
