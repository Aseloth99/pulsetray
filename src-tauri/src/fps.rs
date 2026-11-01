//! Frames per second of the app in front, for the mini gauge.
//!
//! Windows' graphics stack writes an ETW event for every frame an app hands over for display (the same events
//! PresentMon and the Xbox Game Bar count): DXGI for Direct3D 10/11/12, D3D9, and the graphics kernel's present
//! history for Vulkan and OpenGL. A small real-time trace session counts them per process; nothing is injected
//! into the game. Starting a trace session needs an administrator or a member of "Performance Log Users".
//!
//! DXGI and D3D9 are cheap (only their present events are switched on). The kernel's present history can only be
//! had with its "Base" events, which made the graphics driver write ~40 000 events a second while a DirectX 12
//! game ran — so the kernel is listened to only for an app in front that draws without DirectX (`KernelPlan`).
//!
//! The session only runs while the gauge shows FPS, and is stopped when PulseTray quits (a session left behind
//! by a crash is replaced on the next start).

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FpsReading {
    /// Frames per second of the app in front; None while it draws nothing (desktop, a document, a paused app).
    pub fps: Option<f32>,
    /// Average time between those frames (ms).
    pub frame_ms: Option<f32>,
    /// The app in front (exe name without ".exe"), empty for none.
    pub app: String,
    /// Why there is no measurement at all: "denied" (Windows refused the trace session), "failed"; empty when running.
    pub error: String,
}

/// Where a present was seen. Lower wins for a process seen in several (a Direct3D app also shows in the kernel).
const SOURCES: usize = 4;
const SRC_DXGI: usize = 0;
const SRC_D3D9: usize = 1;
const SRC_KERNEL_DETAILED: usize = 2;
const SRC_KERNEL: usize = 3;

/// Event times are FILETIME units (100 ns).
const SECOND: i64 = 10_000_000;
/// FPS is counted over the last second of frames...
const WINDOW: i64 = SECOND;
/// ...kept a little longer, and believed only while frames keep coming (ETW hands real-time events over in
/// buffers, up to about a second late).
const KEEP: i64 = 2 * SECOND;
const STALE: i64 = 3 * SECOND;

#[derive(Default)]
struct Presents {
    by_pid: HashMap<u32, [VecDeque<i64>; SOURCES]>,
}

static PRESENTS: Mutex<Option<Presents>> = Mutex::new(None);

impl Presents {
    fn add(&mut self, pid: u32, src: usize, t: i64) {
        let q = &mut self.by_pid.entry(pid).or_default()[src];
        q.push_back(t);
        while q.front().is_some_and(|&f| f < t - KEEP) {
            q.pop_front();
        }
        // a frame rate in the thousands still fits; a runaway queue doesn't
        while q.len() > 4000 {
            q.pop_front();
        }
    }

    /// Drop processes that stopped presenting.
    fn prune(&mut self, now: i64) {
        self.by_pid.retain(|_, s| s.iter().any(|q| q.back().is_some_and(|&t| t > now - STALE)));
    }
}

/// Frames per second from present times (oldest first): the frames over the last second up to the newest one.
/// None for fewer than 3 frames there or when the newest is stale.
fn rate(times: &VecDeque<i64>, now: i64) -> Option<f32> {
    let &last = times.back()?;
    if last < now - STALE {
        return None;
    }
    let first_in = times.iter().position(|&t| t >= last - WINDOW)?;
    let n = times.len() - first_in;
    if n < 3 {
        return None;
    }
    let span = (last - times[first_in]) as f64 / SECOND as f64;
    (span > 0.0).then(|| ((n - 1) as f64 / span) as f32)
}

/// The frame rate of one process: through DirectX (DXGI, D3D9) and as the kernel saw it.
fn rates_of(sources: &[VecDeque<i64>; SOURCES], now: i64) -> (Option<f32>, Option<f32>) {
    let api = sources[..SRC_KERNEL_DETAILED].iter().find_map(|q| rate(q, now));
    let kernel = sources[SRC_KERNEL_DETAILED..].iter().find_map(|q| rate(q, now));
    (api, kernel)
}

/// How long the kernel is listened to for an app in front before deciding it draws nothing (events come up to a
/// second late), and how long such an app is then left alone.
const PROBE: Duration = Duration::from_secs(4);
const SKIP_FOR: Duration = Duration::from_secs(300);

/// Whether to listen to the graphics kernel (see the module doc): for the app in front when it shows no DirectX
/// frames — a few seconds to look, then for as long as it stays in front if it drew frames (Vulkan, OpenGL).
#[derive(Default)]
struct KernelPlan {
    /// The app being listened for: since when, and whether frames were found.
    target: Option<(u32, Instant, bool)>,
    /// Apps looked at without finding frames (desktop, a browser...), not tried again until then.
    skip: HashMap<u32, Instant>,
}

impl KernelPlan {
    fn decide(&mut self, pid: u32, api_frames: bool, kernel_frames: bool, now: Instant) -> bool {
        if pid == 0 || pid == std::process::id() || api_frames {
            self.target = None;
            return false;
        }
        self.skip.retain(|_, until| *until > now);
        match self.target {
            Some((p, since, found)) if p == pid => {
                let found = found || kernel_frames;
                if found || now.duration_since(since) < PROBE {
                    self.target = Some((p, since, found));
                    true
                } else {
                    self.skip.insert(pid, now + SKIP_FOR);
                    self.target = None;
                    false
                }
            }
            _ if self.skip.contains_key(&pid) => {
                self.target = None;
                false
            }
            _ => {
                self.target = Some((pid, now, kernel_frames));
                true
            }
        }
    }
}

pub struct FpsMeter {
    session: String,
    #[cfg(windows)]
    running: Option<imp::Running>,
    error: String,
    /// A refused start is not retried every second.
    retry_at: Option<std::time::Instant>,
    /// The process in front and its name (looked up when it changes).
    front: (u32, String),
    plan: KernelPlan,
    /// PulseTray is quitting: the session must not be started again (it would outlive the process).
    closed: bool,
}

impl FpsMeter {
    /// `session`: the trace session's name (one per app identifier, so a test build doesn't stop the real one's).
    pub fn new(session: String) -> Self {
        Self {
            session,
            #[cfg(windows)]
            running: None,
            error: String::new(),
            retry_at: None,
            front: (0, String::new()),
            plan: KernelPlan::default(),
            closed: false,
        }
    }

    /// Stop for good (on quit).
    pub fn close(&mut self) {
        self.set_running(false);
        self.closed = true;
    }

    /// Start or stop the trace session.
    pub fn set_running(&mut self, on: bool) {
        #[cfg(windows)]
        {
            if !on {
                if let Some(r) = self.running.take() {
                    r.stop();
                }
                *PRESENTS.lock().unwrap() = None;
                self.error.clear();
                self.retry_at = None;
                self.plan = KernelPlan::default();
                return;
            }
            // a session stopped from outside (another tool, `logman stop`) ended the reading thread: start again
            if self.running.as_ref().is_some_and(|r| r.ended()) {
                if let Some(r) = self.running.take() {
                    r.stop();
                }
            }
            if self.closed || self.running.is_some() || self.retry_at.is_some_and(|t| Instant::now() < t) {
                return;
            }
            *PRESENTS.lock().unwrap() = Some(Presents::default());
            match imp::Running::start(&self.session) {
                Ok(r) => {
                    self.running = Some(r);
                    self.error.clear();
                    self.retry_at = None;
                }
                Err(e) => {
                    self.error = e;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = on;
            self.error = "failed".into();
        }
    }

    /// The frame rate of the app in front.
    pub fn read(&mut self) -> FpsReading {
        if !self.error.is_empty() {
            return FpsReading { error: self.error.clone(), ..Default::default() };
        }
        let pid = imp::foreground_pid();
        if pid != self.front.0 {
            let name = crate::apps::exe_path(pid)
                .and_then(|p| std::path::Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_default();
            self.front = (pid, name);
        }
        let now = imp::now_filetime();
        let (api, kernel) = {
            let mut guard = PRESENTS.lock().unwrap();
            guard
                .as_mut()
                .map(|p| {
                    p.prune(now);
                    p.by_pid.get(&pid).map(|s| rates_of(s, now)).unwrap_or_default()
                })
                .unwrap_or_default()
        };
        let listen = self.plan.decide(pid, api.is_some(), kernel.is_some(), Instant::now());
        #[cfg(windows)]
        if let Some(r) = self.running.as_mut() {
            r.set_kernel(listen);
        }
        let fps = api.or(if listen { kernel } else { None });
        FpsReading {
            fps: fps.map(|f| (f * 10.0).round() / 10.0),
            frame_ms: fps.filter(|&f| f > 0.0).map(|f| ((1000.0 / f) * 10.0).round() / 10.0),
            app: self.front.1.clone(),
            error: String::new(),
        }
    }

    /// Frames per second of every process seen presenting (for the live test).
    #[cfg(test)]
    fn all_rates(&self) -> Vec<(u32, usize, f32)> {
        let now = imp::now_filetime();
        let guard = PRESENTS.lock().unwrap();
        let Some(p) = guard.as_ref() else { return vec![] };
        let mut out: Vec<(u32, usize, f32)> = p
            .by_pid
            .iter()
            .filter_map(|(&pid, s)| s.iter().enumerate().find_map(|(i, q)| rate(q, now).map(|f| (pid, i, f))))
            .collect();
        out.sort_by(|a, b| b.2.total_cmp(&a.2));
        out
    }
}

impl Drop for FpsMeter {
    fn drop(&mut self) {
        self.set_running(false);
    }
}

#[cfg(windows)]
mod imp {
    use super::{Presents, PRESENTS, SRC_D3D9, SRC_DXGI, SRC_KERNEL, SRC_KERNEL_DETAILED};
    use windows::core::{GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Diagnostics::Etw::{
        CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, CONTROLTRACE_HANDLE,
        ENABLE_TRACE_PARAMETERS, ENABLE_TRACE_PARAMETERS_VERSION_2, EVENT_CONTROL_CODE_DISABLE_PROVIDER,
        EVENT_CONTROL_CODE_ENABLE_PROVIDER,
        EVENT_FILTER_DESCRIPTOR, EVENT_FILTER_TYPE_EVENT_ID, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
        EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE, PROCESSTRACE_HANDLE,
        PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, WNODE_FLAG_TRACED_GUID,
    };

    /// Microsoft-Windows-DXGI: Present_Start (42), PresentMultiplaneOverlay_Start (55).
    const DXGI: GUID = GUID::from_u128(0xca11c036_0102_4a2d_a6ad_f03cfed5d3c9);
    /// Microsoft-Windows-D3D9: Present_Start (1).
    const D3D9: GUID = GUID::from_u128(0x783aca0a_790e_4d7f_8451_aa850511c6b9);
    /// Microsoft-Windows-DxgKrnl: PresentHistoryDetailed_Start (215), PresentHistory_Start (171) — every
    /// present that reaches the kernel, which is how Vulkan and OpenGL frames show up.
    const DXGKRNL: GUID = GUID::from_u128(0x802ec45a_1e99_4b83_9920_87c98277ba9d);
    /// DxgKrnl "Base": the narrowest keyword holding the present history (it also holds the packet and fence
    /// events, hence `KernelPlan`).
    const DXGKRNL_BASE: u64 = 0x1;
    /// DXGI and D3D9 "Events": present and flip only (keyword 0 would add every API call, ~1 700 events/s).
    const API_EVENTS: u64 = 0x2;
    const TRACE_LEVEL_INFORMATION: u8 = 4;

    unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
        let Some(rec) = (unsafe { rec.as_ref() }) else { return };
        let h = &rec.EventHeader;
        let id = h.EventDescriptor.Id;
        let src = if h.ProviderId == DXGI && (id == 42 || id == 55) {
            SRC_DXGI
        } else if h.ProviderId == D3D9 && id == 1 {
            SRC_D3D9
        } else if h.ProviderId == DXGKRNL && id == 215 {
            SRC_KERNEL_DETAILED
        } else if h.ProviderId == DXGKRNL && id == 171 {
            SRC_KERNEL
        } else {
            return;
        };
        if let Some(p) = PRESENTS.lock().unwrap().as_mut() {
            Presents::add(p, h.ProcessId, src, h.TimeStamp);
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// EVENT_TRACE_PROPERTIES followed by room for the session name (as StartTrace / ControlTrace want it).
    fn properties(name_len: usize) -> Vec<u64> {
        let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + (name_len + 1) * 2;
        let mut buf = vec![0u64; size.div_ceil(8)];
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        unsafe {
            (*p).Wnode.BufferSize = size as u32;
            (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*p).Wnode.ClientContext = 1; // QueryPerformanceCounter time stamps
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            (*p).BufferSize = 16; // KB: a frame is a few dozen bytes
            (*p).MinimumBuffers = 4;
            (*p).MaximumBuffers = 32;
            (*p).FlushTimer = 1; // hand events over at least every second
            (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        }
        buf
    }

    fn enable(handle: CONTROLTRACE_HANDLE, provider: &GUID, keywords: u64, ids: &[u16]) -> WIN32_ERROR {
        // EVENT_FILTER_EVENT_ID: FilterIn (1 byte), Reserved, Count (u16), then the ids.
        let mut filter = vec![0u8; 4 + 2 * ids.len()];
        filter[0] = 1;
        filter[2..4].copy_from_slice(&(ids.len() as u16).to_le_bytes());
        for (i, id) in ids.iter().enumerate() {
            filter[4 + 2 * i..6 + 2 * i].copy_from_slice(&id.to_le_bytes());
        }
        let mut desc =
            EVENT_FILTER_DESCRIPTOR { Ptr: filter.as_ptr() as u64, Size: filter.len() as u32, Type: EVENT_FILTER_TYPE_EVENT_ID };
        let params = ENABLE_TRACE_PARAMETERS {
            Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
            EnableFilterDesc: &mut desc,
            FilterDescCount: 1,
            ..Default::default()
        };
        unsafe {
            EnableTraceEx2(
                handle,
                provider,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                TRACE_LEVEL_INFORMATION,
                keywords,
                0,
                0,
                Some(&params),
            )
        }
    }

    fn stop_by_name(name: &[u16]) {
        let mut props = properties(name.len());
        unsafe {
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(name.as_ptr()),
                props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_CONTROL_STOP,
            );
        }
    }

    pub struct Running {
        name: Vec<u16>,
        control: CONTROLTRACE_HANDLE,
        thread: Option<std::thread::JoinHandle<()>>,
        kernel: bool,
    }

    impl Running {
        pub fn start(session: &str) -> Result<Self, String> {
            let name = wide(session);
            let mut control = CONTROLTRACE_HANDLE::default();
            let mut props = properties(name.len());
            let mut err = unsafe {
                StartTraceW(&mut control, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES)
            };
            if err == ERROR_ALREADY_EXISTS {
                // left behind by a crash (or a killed process): take it over
                stop_by_name(&name);
                props = properties(name.len());
                err = unsafe {
                    StartTraceW(&mut control, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES)
                };
            }
            if err == ERROR_ACCESS_DENIED {
                return Err("denied".into());
            }
            if err != ERROR_SUCCESS {
                return Err("failed".into());
            }
            let enabled = [enable(control, &DXGI, API_EVENTS, &[42, 55]), enable(control, &D3D9, API_EVENTS, &[1])];
            if enabled.iter().all(|e| *e != ERROR_SUCCESS) {
                stop_by_name(&name);
                return Err(if enabled.contains(&ERROR_ACCESS_DENIED) { "denied" } else { "failed" }.into());
            }

            let mut logger = name.clone();
            let mut logfile = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(logger.as_mut_ptr()), ..Default::default() };
            logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            logfile.Anonymous2.EventRecordCallback = Some(on_event);
            let trace = unsafe { OpenTraceW(&mut logfile) };
            if trace.Value == u64::MAX {
                stop_by_name(&name);
                return Err("failed".into());
            }
            let thread = std::thread::Builder::new()
                .name("fps-trace".into())
                .spawn(move || {
                    let _keep = logger; // the name OpenTrace was given
                    unsafe {
                        let _ = ProcessTrace(&[trace], None, None);
                        let _ = CloseTrace(PROCESSTRACE_HANDLE { Value: trace.Value });
                    }
                })
                .map_err(|_| {
                    unsafe {
                        let _ = CloseTrace(trace);
                    }
                    stop_by_name(&name);
                    "failed".to_string()
                })?;
            Ok(Self { name, control, thread: Some(thread), kernel: false })
        }

        /// Listen to the graphics kernel's present history too (or stop).
        pub fn set_kernel(&mut self, on: bool) {
            if on == self.kernel {
                return;
            }
            let ok = if on {
                enable(self.control, &DXGKRNL, DXGKRNL_BASE, &[171, 215]) == ERROR_SUCCESS
            } else {
                let _ = unsafe {
                    EnableTraceEx2(self.control, &DXGKRNL, EVENT_CONTROL_CODE_DISABLE_PROVIDER.0, 0, 0, 0, 0, None)
                };
                true
            };
            if ok {
                self.kernel = on;
            }
        }

        /// The reading thread is gone (the session was stopped from outside).
        pub fn ended(&self) -> bool {
            self.thread.as_ref().is_none_or(|t| t.is_finished())
        }

        pub fn stop(mut self) {
            let mut props = properties(self.name.len());
            unsafe {
                let _ = ControlTraceW(
                    self.control,
                    PCWSTR::null(),
                    props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                    EVENT_TRACE_CONTROL_STOP,
                );
            }
            // ProcessTrace returns once the session is gone.
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Test only: how many events a provider writes per id over `secs` with no event filter (what enabling it
    /// costs the drivers, not just what reaches us).
    #[cfg(test)]
    pub fn count_unfiltered(provider: GUID, keywords: u64, secs: u64) -> Result<Vec<(u16, u64)>, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTS: [AtomicU64; 2048] = [const { AtomicU64::new(0) }; 2048];
        unsafe extern "system" fn count(rec: *mut EVENT_RECORD) {
            if let Some(rec) = unsafe { rec.as_ref() } {
                let id = rec.EventHeader.EventDescriptor.Id as usize;
                COUNTS[id.min(2047)].fetch_add(1, Ordering::Relaxed);
            }
        }
        let name = wide("PulseTray FPS cost test");
        stop_by_name(&name);
        let mut control = CONTROLTRACE_HANDLE::default();
        let mut props = properties(name.len());
        let err = unsafe {
            StartTraceW(&mut control, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES)
        };
        if err != ERROR_SUCCESS {
            return Err(format!("start {err:?}"));
        }
        let err = unsafe {
            EnableTraceEx2(control, &provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, TRACE_LEVEL_INFORMATION, keywords, 0, 0, None)
        };
        if err != ERROR_SUCCESS {
            stop_by_name(&name);
            return Err(format!("enable {err:?}"));
        }
        let mut logger = name.clone();
        let mut logfile = EVENT_TRACE_LOGFILEW { LoggerName: PWSTR(logger.as_mut_ptr()), ..Default::default() };
        logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(count);
        let trace = unsafe { OpenTraceW(&mut logfile) };
        let t = std::thread::spawn(move || unsafe {
            let _ = ProcessTrace(&[trace], None, None);
            let _ = CloseTrace(trace);
        });
        std::thread::sleep(std::time::Duration::from_secs(secs));
        stop_by_name(&name);
        let _ = t.join();
        let mut out: Vec<(u16, u64)> = (0..2048)
            .filter_map(|i| {
                let n = COUNTS[i].swap(0, Ordering::Relaxed);
                (n > 0).then_some((i as u16, n / secs))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(out)
    }

    /// The process of the window in front. A Microsoft Store app's window belongs to ApplicationFrameHost, while
    /// the app (and its frames) live in the process of the CoreWindow inside it.
    pub fn foreground_pid() -> u32 {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowExW, GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
        };
        let mut pid = 0u32;
        unsafe {
            let w = GetForegroundWindow();
            if w.is_invalid() {
                return 0;
            }
            GetWindowThreadProcessId(w, Some(&mut pid));
            let mut class = [0u16; 64];
            let n = GetClassNameW(w, &mut class) as usize;
            if String::from_utf16_lossy(&class[..n]) == "ApplicationFrameWindow" {
                if let Ok(core) = FindWindowExW(Some(w), None, w!("Windows.UI.Core.CoreWindow"), None) {
                    let mut app = 0u32;
                    GetWindowThreadProcessId(core, Some(&mut app));
                    if app != 0 {
                        pid = app;
                    }
                }
            }
        }
        pid
    }

    /// Now, in the units of the event time stamps (FILETIME: 100 ns since 1601).
    pub fn now_filetime() -> i64 {
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        (unix.as_nanos() / 100) as i64 + 116_444_736_000_000_000
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn foreground_pid() -> u32 {
        0
    }
    pub fn now_filetime() -> i64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(fps: f64, secs: f64, end: i64) -> VecDeque<i64> {
        let n = (fps * secs) as i64;
        let step = (SECOND as f64 / fps) as i64;
        (0..n).map(|i| end - (n - 1 - i) * step).collect()
    }

    #[test]
    fn the_rate_is_the_frames_over_the_last_second() {
        let now = 1_000 * SECOND;
        let r = rate(&times(144.0, 2.0, now), now).unwrap();
        assert!((r - 144.0).abs() < 1.0, "{r}");
        let r = rate(&times(30.0, 2.0, now), now).unwrap();
        assert!((r - 30.0).abs() < 0.5, "{r}");
    }

    #[test]
    fn frames_that_stopped_coming_are_no_rate() {
        let now = 1_000 * SECOND;
        let q = times(60.0, 1.0, now - 4 * SECOND);
        assert_eq!(rate(&q, now), None);
        // a couple of stray presents are no frame rate either
        let q: VecDeque<i64> = [now - SECOND / 2, now].into_iter().collect();
        assert_eq!(rate(&q, now), None);
    }

    #[test]
    fn the_api_the_app_uses_wins_over_the_kernel_copy() {
        let now = 1_000 * SECOND;
        let mut p = Presents::default();
        for t in times(60.0, 1.5, now) {
            p.add(7, SRC_DXGI, t);
        }
        for t in times(120.0, 1.5, now) {
            p.add(7, SRC_KERNEL, t);
        }
        let (api, kernel) = rates_of(&p.by_pid[&7], now);
        assert!((api.unwrap() - 60.0).abs() < 1.0, "{api:?}");
        assert!(kernel.is_some());
        // only the kernel saw it (Vulkan / OpenGL)
        for t in times(90.0, 1.5, now) {
            p.add(8, SRC_KERNEL_DETAILED, t);
        }
        let (api, kernel) = rates_of(&p.by_pid[&8], now);
        assert!(api.is_none() && (kernel.unwrap() - 90.0).abs() < 1.0, "{kernel:?}");
    }

    #[test]
    fn the_kernel_is_only_listened_to_for_an_app_in_front_drawing_without_directx() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut k = KernelPlan::default();
        // a DirectX game in front: never
        assert!(!k.decide(10, true, false, at(0)));
        // an app without DirectX frames: a look of a few seconds...
        assert!(k.decide(20, false, false, at(1)));
        assert!(k.decide(20, false, false, at(4)));
        // ...nothing found: off, and not tried again for a while
        assert!(!k.decide(20, false, false, at(6)));
        assert!(!k.decide(20, false, false, at(60)));
        assert!(k.decide(20, false, false, at(7 + 300)));
        // a Vulkan / OpenGL game: frames found, listened to while it stays in front, even through a pause
        assert!(k.decide(30, false, false, at(400)));
        assert!(k.decide(30, false, true, at(402)));
        assert!(k.decide(30, false, false, at(420)));
        // another app in front: a new look; the game again: a new look too (and found again)
        assert!(k.decide(40, false, false, at(421)));
        assert!(k.decide(30, false, true, at(422)));
        // our own popup in front: never
        assert!(!k.decide(std::process::id(), false, false, at(423)));
    }

    #[test]
    fn old_frames_are_dropped_and_quiet_processes_pruned() {
        let now = 1_000 * SECOND;
        let mut p = Presents::default();
        for t in times(100.0, 5.0, now) {
            p.add(1, SRC_DXGI, t);
        }
        assert!(p.by_pid[&1][SRC_DXGI].len() <= 201, "{}", p.by_pid[&1][SRC_DXGI].len());
        p.add(2, SRC_DXGI, now - 10 * SECOND);
        p.prune(now);
        assert!(p.by_pid.contains_key(&1) && !p.by_pid.contains_key(&2));
    }

    /// Live: events per second each provider would write with its keyword and no id filter.
    /// `cargo test --lib fps::tests::live_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_cost() {
        use windows::core::GUID;
        for (name, guid, kw) in [
            ("DXGI Events", GUID::from_u128(0xca11c036_0102_4a2d_a6ad_f03cfed5d3c9), 0x2u64),
            ("D3D9 Events", GUID::from_u128(0x783aca0a_790e_4d7f_8451_aa850511c6b9), 0x2),
            ("DxgKrnl Base", GUID::from_u128(0x802ec45a_1e99_4b83_9920_87c98277ba9d), 0x1),
        ] {
            let r = imp::count_unfiltered(guid, kw, 3).unwrap();
            let total: u64 = r.iter().map(|x| x.1).sum();
            println!("{name}: {total} events/s; top {:?}", &r[..r.len().min(12)]);
        }
    }

    /// Live: a session stopped from outside is noticed and started again.
    /// `cargo test --lib fps::tests::live_restart -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_restart() {
        let name = "PulseTray FPS restart test";
        let mut m = FpsMeter::new(name.into());
        m.set_running(true);
        assert!(m.error.is_empty(), "{}", m.error);
        let out = std::process::Command::new("logman").args(["stop", name, "-ets"]).output().unwrap();
        println!("logman stop: {}", String::from_utf8_lossy(&out.stdout).trim());
        std::thread::sleep(Duration::from_millis(500));
        assert!(m.running.as_ref().unwrap().ended(), "the reading thread ended with the session");
        m.set_running(true);
        assert!(!m.running.as_ref().unwrap().ended(), "started again");
        let q = std::process::Command::new("logman").args(["query", "-ets"]).output().unwrap();
        assert!(String::from_utf8_lossy(&q.stdout).contains(name));
        m.close();
        let q = std::process::Command::new("logman").args(["query", "-ets"]).output().unwrap();
        assert!(!String::from_utf8_lossy(&q.stdout).contains(name), "gone after close");
    }

    /// Live: runs the trace session for a few seconds and prints who presents how fast.
    /// `cargo test --lib fps::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live() {
        let mut m = FpsMeter::new("PulseTray FPS test".into());
        let t0 = std::time::Instant::now();
        m.set_running(true);
        println!("start: {:?} error={:?}", t0.elapsed(), m.error);
        std::thread::sleep(std::time::Duration::from_secs(4));
        for (pid, src, fps) in m.all_rates().into_iter().take(12) {
            let name = crate::apps::exe_path(pid).unwrap_or_default();
            println!("{pid:>6} src{src} {fps:>7.1} fps  {name}");
        }
        println!("front: {:?}", m.read());
        let t1 = std::time::Instant::now();
        m.set_running(false);
        println!("stop: {:?}", t1.elapsed());
        assert!(m.error.is_empty());
    }
}
