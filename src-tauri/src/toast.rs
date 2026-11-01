//! System notifications for the alerts.
//!
//! Windows (the shipped build): toasts through WinRT. PulseTray is mostly run as a portable exe with no Start-menu
//! shortcut, and Windows only shows toasts for an app it knows. A desktop app can introduce itself under
//! HKCU\Software\Classes\AppUserModelId\<id> (name and icon), which is what Microsoft's own toolkit does for unpackaged
//! apps; that key is written the first time a toast is shown. A click on a toast that is still on screen opens the popup.
//!
//! macOS / Linux (built only there, never run on one yet): the `notify-rust` crate, which talks to the system's own
//! notification service (Notification Center / the freedesktop notification daemon). It has no click handler here:
//! the panel opens from the tray icon.

use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use windows::core::{HSTRING, PCWSTR};
#[cfg(windows)]
use windows::Data::Xml::Dom::XmlDocument;
#[cfg(windows)]
use windows::Foundation::TypedEventHandler;
#[cfg(windows)]
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
#[cfg(windows)]
use windows::Win32::Foundation::ERROR_SUCCESS;
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SZ,
};

/// Toasts go out under the app identifier (tauri.conf.json), the id the installer's shortcut carries too: installed
/// and portable copies share one entry in Windows' notification settings, a test build has its own.
pub fn app_id(app: &tauri::AppHandle) -> String {
    app.config().identifier.clone()
}
const ICON_PNG: &[u8] = include_bytes!("../icons/128x128.png");

/// Toasts that may still be clicked: dropping one would drop its click handler.
#[cfg(windows)]
static LIVE: Mutex<Vec<ToastNotification>> = Mutex::new(Vec::new());

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The toast's XML: a title and up to a few lines of text.
#[cfg(windows)]
pub fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast activationType=\"foreground\" launch=\"pulsetray\"><visual><binding template=\"ToastGeneric\">\
         <text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    )
}

/// HKCU\Software\Classes\AppUserModelId\<id>: DisplayName, IconUri.
#[cfg(windows)]
fn register(id: &str, name: &str, icon: Option<&Path>) -> Result<(), String> {
    let sub = wide(&format!("Software\\Classes\\AppUserModelId\\{id}"));
    let mut key = HKEY::default();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(format!("registry create failed ({})", rc.0));
    }
    let set = |value: &str, data: &str| {
        let (n, d) = (wide(value), wide(data));
        let bytes = unsafe { std::slice::from_raw_parts(d.as_ptr() as *const u8, d.len() * 2) };
        unsafe { RegSetValueExW(key, PCWSTR(n.as_ptr()), None, REG_SZ, Some(bytes)) }
    };
    let mut ok = set("DisplayName", name) == ERROR_SUCCESS;
    if let Some(icon) = icon {
        ok &= set("IconUri", &icon.display().to_string()) == ERROR_SUCCESS;
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    if ok { Ok(()) } else { Err("registry write failed".into()) }
}

/// The icon shown next to the notification (a file, not an exe resource).
fn icon_file(data_dir: Option<&Path>) -> Option<PathBuf> {
    let path = data_dir?.join("toast-icon.png");
    let fresh = std::fs::metadata(&path).is_ok_and(|m| m.len() == ICON_PNG.len() as u64);
    if !fresh {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, ICON_PNG).ok()?;
    }
    Some(path)
}

#[cfg(windows)]
pub struct Toaster {
    id: String,
    registered: bool,
    data_dir: Option<PathBuf>,
}

#[cfg(windows)]
impl Toaster {
    pub fn new(id: &str, data_dir: Option<PathBuf>) -> Self {
        Self { id: id.to_string(), registered: false, data_dir }
    }

    /// Shows a toast; `on_click` runs (on a Windows thread) when it is clicked while on screen.
    pub fn show(&mut self, title: &str, body: &str, on_click: impl Fn() + Send + 'static) -> Result<(), String> {
        if !self.registered {
            let icon = icon_file(self.data_dir.as_deref());
            register(&self.id, "PulseTray", icon.as_deref())?;
            self.registered = true;
        }
        let err = |e: windows::core::Error| e.message();
        let xml = XmlDocument::new().map_err(err)?;
        xml.LoadXml(&HSTRING::from(toast_xml(title, body))).map_err(err)?;
        let toast = ToastNotification::CreateToastNotification(&xml).map_err(err)?;
        toast
            .Activated(&TypedEventHandler::new(move |_, _| {
                on_click();
                Ok(())
            }))
            .map_err(err)?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&self.id)).map_err(err)?;
        notifier.Show(&toast).map_err(err)?;
        let mut live = LIVE.lock().unwrap();
        live.push(toast);
        let excess = live.len().saturating_sub(8);
        live.drain(..excess);
        Ok(())
    }
}

/// macOS / Linux: the system's own notification service through `notify-rust`.
#[cfg(not(windows))]
pub struct Toaster {
    data_dir: Option<PathBuf>,
}

#[cfg(not(windows))]
impl Toaster {
    pub fn new(_id: &str, data_dir: Option<PathBuf>) -> Self {
        Self { data_dir }
    }

    /// Shows a notification. `on_click` is not used here: `notify-rust` has no click handler on macOS and the
    /// Linux one needs the daemon to support actions, so the panel opens from the tray icon instead.
    pub fn show(&mut self, title: &str, body: &str, on_click: impl Fn() + Send + 'static) -> Result<(), String> {
        let _ = on_click;
        let mut n = notify_rust::Notification::new();
        n.appname("PulseTray").summary(title).body(body);
        if let Some(icon) = icon_file(self.data_dir.as_deref()) {
            n.icon(&icon.display().to_string());
        }
        n.show().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn text_is_escaped_into_the_xml() {
        let x = toast_xml("Disk <C:> dolu", "\"a\" & b");
        assert!(x.contains("<text>Disk &lt;C:&gt; dolu</text><text>&quot;a&quot; &amp; b</text>"), "{x}");
        assert!(XmlDocument::new().unwrap().LoadXml(&HSTRING::from(x)).is_ok(), "well-formed");
    }

    /// The icon file the notification points at is written once and not rewritten while it is current.
    #[test]
    fn the_icon_file_is_written_once() {
        let dir = std::env::temp_dir().join(format!("pulsetray-icon-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = icon_file(Some(&dir)).expect("written");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), ICON_PNG.len() as u64);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(icon_file(Some(&dir)).as_deref(), Some(path.as_path()));
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before, "not rewritten");
        assert_eq!(icon_file(None), None, "no folder, no icon");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Shows a real toast under a throw-away id, checks that Windows took it, then removes it and the id.
    #[cfg(windows)]
    #[test]
    #[ignore = "shows a notification on screen: run with --ignored --nocapture"]
    fn live_toast() {
        let id = "com.pulsetray.toasttest";
        let dir = std::env::temp_dir().join("pulsetray-toast-test");
        let mut t = Toaster::new(id, Some(dir.clone()));
        t.show("PulseTray deneme", "Bildirimler çalışıyor (test, kendiliğinden silinir).", || {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let history = ToastNotificationManager::History().unwrap();
        let n = history.GetHistoryWithId(&HSTRING::from(id)).unwrap().Size().unwrap();
        println!("in the notification history: {n}");
        let _ = history.ClearWithId(&HSTRING::from(id));
        let sub = wide(&format!("Software\\Classes\\AppUserModelId\\{id}"));
        unsafe {
            let _ = windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()));
        }
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(n, 1);
    }
}
