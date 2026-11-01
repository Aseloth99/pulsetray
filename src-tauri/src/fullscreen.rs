//! "Is a full-screen app (game, video, presentation) in the foreground?"
//!
//! Starts from the shell query Windows uses to decide whether to suppress notifications. Its
//! "busy" answer is also given for ordinary maximized or borderless-looking windows, so it is only
//! believed when a real window on screen covers a whole monitor without a title bar.

#[cfg(windows)]
pub fn is_fullscreen_app() -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) if s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE => true,
        Ok(s) if s == QUNS_BUSY => covering_window_exists(),
        _ => false,
    }
}

/// The top visible window of real size that is not ours (PulseTray's popup, tool windows and small overlays
/// may be in front) fills its monitor and has no caption: a game, a video player in full screen, a slideshow.
#[cfg(windows)]
fn covering_window_exists() -> bool {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_CAPTION, WS_EX_TOOLWINDOW,
    };

    struct Found(bool);
    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam.0 as *mut Found);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
        if cloaked != 0 {
            return BOOL(1);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return BOOL(1);
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        // Desktop, taskbar and cloaked shell hosts are always "full size"; they are not apps.
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
            return BOOL(1);
        }
        // The first window of real size from the top decides (small overlays such as a game's FPS counter or
        // a chat bubble may sit above the game): it either covers a whole monitor without a title bar, or not.
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return BOOL(1);
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return BOOL(1);
        }
        let area = (r.right - r.left).max(0) as i64 * (r.bottom - r.top).max(0) as i64;
        let monitor_area = (mi.rcMonitor.right - mi.rcMonitor.left) as i64 * (mi.rcMonitor.bottom - mi.rcMonitor.top) as i64;
        if area * 4 < monitor_area {
            return BOOL(1);
        }
        let covers = r.left <= mi.rcMonitor.left
            && r.top <= mi.rcMonitor.top
            && r.right >= mi.rcMonitor.right
            && r.bottom >= mi.rcMonitor.bottom;
        found.0 = covers && style & WS_CAPTION.0 != WS_CAPTION.0;
        BOOL(0)
    }

    let mut found = Found(false);
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut found as *mut Found as isize));
    }
    found.0
}

#[cfg(not(windows))]
pub fn is_fullscreen_app() -> bool {
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_does_not_panic() {
        // The answer depends on what is on screen; we only check the call itself is sound.
        let _ = super::is_fullscreen_app();
    }
}
