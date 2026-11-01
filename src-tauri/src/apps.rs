#![allow(unsafe_op_in_unsafe_fn)]
//! What the "Top apps" list can do with a program: show its icon, open its file location, end it.

use crate::procs;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

/// Icons are drawn 16 px tall in the list; 32 px stays sharp up to 200 % display scaling.
const ICON_PX: u32 = 32;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    /// The exe, when Windows tells (not for protected / system processes like vmmem or System).
    pub path: Option<String>,
    /// `data:image/png;base64,...`
    pub icon: Option<String>,
}

/// Icon per exe path (None = the file has none). Extracting one takes a few milliseconds; the list repeats.
static ICONS: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Name, path and icon for each app the list shows, from the first of its pids Windows answers for (the
/// others may have exited already). Without any, the app just comes back without a path.
pub fn infos(apps: &[(String, Vec<u32>)]) -> Vec<AppInfo> {
    apps.iter()
        .map(|(name, pids)| {
            let path = pids.iter().take(8).find_map(|&p| exe_path(p));
            let icon = path.as_ref().and_then(|p| {
                let mut cache = ICONS.lock().unwrap();
                let cache = cache.get_or_insert_with(HashMap::new);
                cache.entry(p.clone()).or_insert_with(|| icon_png(p, ICON_PX).map(|png| data_url(&png))).clone()
            });
            AppInfo { name: name.clone(), path, icon }
        })
        .collect()
}

/// Opens Explorer with the program's file selected (the first of the pids whose path is known).
pub fn reveal(pids: &[u32]) -> bool {
    let Some(path) = pids.iter().find_map(|&p| exe_path(p)) else { return false };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer wants exactly `/select,"C:\path"`; the normal argument quoting would wrap the whole thing.
        std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn().is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EndResult {
    pub ended: u32,
    /// Windows refused (no rights, protected or critical process).
    pub failed: u32,
    /// PulseTray itself or one of its own processes (its web view): never ended from here.
    pub own: u32,
}

/// Ends the processes of one app. The list on screen is a few seconds old and pids are reused, so every pid
/// is checked against the process table right now: it must still carry `name`. PulseTray's own process tree
/// and processes Windows marks as critical are left alone.
pub fn end(name: &str, pids: &[u32]) -> EndResult {
    let mut r = EndResult::default();
    let Some(list) = procs::list_now() else {
        r.failed = pids.len() as u32;
        return r;
    };
    let own = procs::descendants(&list, std::process::id());
    for &pid in pids {
        let Some(p) = list.iter().find(|p| p.0 == pid) else { continue }; // already gone
        if !p.3.eq_ignore_ascii_case(name) || pid <= 4 {
            continue; // another program has this pid now (or the idle / System process)
        }
        if own.contains(&pid) {
            r.own += 1;
        } else if terminate(pid) {
            r.ended += 1;
        } else {
            r.failed += 1;
        }
    }
    r
}

#[cfg(windows)]
fn terminate(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        IsProcessCritical, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut critical = windows::core::BOOL(0);
        // Ending a critical process (csrss, wininit...) blue-screens the machine; Windows only lets an admin try.
        let ok = IsProcessCritical(h, &mut critical).is_ok() && !critical.as_bool() && TerminateProcess(h, 1).is_ok();
        let _ = CloseHandle(h);
        ok
    }
}

#[cfg(not(windows))]
fn terminate(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
pub fn exe_path(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid <= 4 {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        (ok && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(not(windows))]
pub fn exe_path(_pid: u32) -> Option<String> {
    None
}

fn data_url(png: &[u8]) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

pub(crate) fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// The exe's own icon at `size` px as PNG; None when it has none.
#[cfg(windows)]
fn icon_png(path: &str, size: u32) -> Option<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SHDefExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let mut icon = HICON::default();
    unsafe {
        let r = SHDefExtractIconW(PCWSTR(wide.as_ptr()), 0, 0, Some(&mut icon), None, size);
        if r.is_err() || icon.is_invalid() {
            return None;
        }
        let px = icon_rgba(icon);
        let _ = DestroyIcon(icon);
        let (rgba, w, h) = px?;
        encode_png(&rgba, w, h)
    }
}

#[cfg(not(windows))]
fn icon_png(_path: &str, _size: u32) -> Option<Vec<u8>> {
    None
}

/// An icon's pixels as straight RGBA. Old icons without an alpha channel get it from their mask.
#[cfg(windows)]
unsafe fn icon_rgba(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<(Vec<u8>, u32, u32)> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let free = |b: HBITMAP| {
        if !b.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(b.0));
        }
    };
    let result = (|| {
        if info.hbmColor.is_invalid() {
            return None; // monochrome icon: not worth the trouble
        }
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(info.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some((&mut bm as *mut BITMAP).cast())) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.max(0) as u32);
        if w == 0 || h == 0 || w > 256 || h > 256 {
            return None;
        }
        let dc = CreateCompatibleDC(None);
        let read = |b: HBITMAP| -> Option<Vec<u8>> {
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32), // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(dc, b, 0, h, Some(px.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
            (lines == h as i32).then_some(px)
        };
        let color = read(info.hbmColor);
        let mask = if info.hbmMask.is_invalid() { None } else { read(info.hbmMask) };
        let _ = DeleteDC(dc);
        let mut px = color?;
        let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            p.swap(0, 2); // BGRA -> RGBA
            if !has_alpha {
                // Mask: black = the icon, white = see-through.
                let transparent = mask.as_ref().is_some_and(|m| m[i * 4] != 0);
                p[3] = if transparent { 0 } else { 255 };
            }
        }
        Some((px, w, h))
    })();
    free(info.hbmColor);
    free(info.hbmMask);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn this_exe_has_a_path_and_pid_4_does_not() {
        let me = exe_path(std::process::id()).expect("own path");
        assert!(me.to_lowercase().ends_with(".exe"), "{me}");
        assert_eq!(exe_path(4), None);
        assert_eq!(exe_path(0), None);
    }

    #[test]
    #[cfg(windows)]
    fn explorer_has_an_icon_that_becomes_a_png() {
        let path = std::env::var("WINDIR").map(|w| format!("{w}\\explorer.exe")).unwrap();
        let png = icon_png(&path, ICON_PX).expect("explorer.exe has an icon");
        assert_eq!(&png[1..4], b"PNG");
        // The first pid has no path (0): the next one answers. (The test binary itself has no icon resource.)
        let info = infos(&[("me.exe".into(), vec![0, std::process::id()])]);
        assert!(info[0].path.is_some(), "{info:?}");
        assert!(info[0].icon.is_none(), "an exe without an icon gets none, not a made-up one");
    }

    #[test]
    #[cfg(windows)]
    fn ending_checks_the_name_and_spares_pulsetray_itself() {
        // Our own pid under the wrong name: another program could have taken a stale pid, so nothing happens.
        let me = std::process::id();
        assert_eq!(end("not-this.exe", &[me]), EndResult::default());
        // Our own pid under its real name: refused as PulseTray's own process.
        let list = procs::list_now().unwrap();
        let name = list.iter().find(|p| p.0 == me).unwrap().3.clone();
        assert_eq!(end(&name, &[me]), EndResult { own: 1, ..Default::default() });
    }

    #[test]
    #[cfg(windows)]
    fn a_child_process_of_ours_is_spared_but_a_foreign_one_ends() {
        use std::process::{Command, Stdio};
        // A child of the test process belongs to "our" tree: spared.
        let mut child = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let r = end("PING.EXE", &[child.id()]);
        assert_eq!(r, EndResult { own: 1, ..Default::default() });
        // Started through `cmd /c start`, the ping's parent (cmd) exits at once: not ours any more.
        let pings = |list: &[(u32, u32, i64, String)]| -> Vec<u32> {
            list.iter().filter(|p| p.3.eq_ignore_ascii_case("PING.EXE")).map(|p| p.0).collect()
        };
        let before = pings(&procs::list_now().unwrap());
        let _ = Command::new("cmd").args(["/c", "start", "/b", "", "ping", "-n", "30", "127.0.0.2"]).stdout(Stdio::null()).status();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let list = procs::list_now().unwrap();
        let ours = procs::descendants(&list, std::process::id());
        // Only the ping started just now (never one the user may be running).
        let orphan = pings(&list).into_iter().find(|p| !before.contains(p) && !ours.contains(p));
        let pid = orphan.expect("the ping started through cmd is running");
        assert_eq!(end("ping.exe", &[pid]), EndResult { ended: 1, ..Default::default() }, "name check ignores case");
        let _ = child.kill();
        let _ = child.wait();
    }
}
