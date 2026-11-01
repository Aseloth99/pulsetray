//! "Start at Login".
//!
//! Windows: one value under HKCU\...\Run. Written by hand instead of through the `auto-launch` crate, because that
//! crate stores the exe path unquoted, which only works for folders with spaces thanks to CreateProcess' guessing.
//!
//! macOS / Linux (the same `is_enabled` / `set_enabled`, only built there): a launch agent
//! (`~/Library/LaunchAgents/<label>.plist`, RunAtLoad) or an XDG autostart entry (`~/.config/autostart/<name>.desktop`).
//! The text of both files and their paths are plain functions below (tested everywhere); only the file handling is
//! compiled for those systems, and has not been run on one. Every start from there carries `--minimized` like the
//! Windows value does.

use std::path::Path;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::ERROR_SUCCESS;
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

#[cfg(windows)]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RELEASE_ID: &str = "com.pulsetray.app";

/// "PulseTray" for the released app. A build with another identifier (a test build) gets its own value, so
/// switching it on (a fresh test profile starts at login by default) never repoints the installed copy.
#[cfg(windows)]
pub fn value_name(identifier: &str) -> String {
    if identifier == RELEASE_ID { "PulseTray".into() } else { format!("PulseTray ({identifier})") }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `"C:\path with spaces\pulsetray.exe" --minimized`
#[cfg(windows)]
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

#[cfg(windows)]
fn open(subkey: &str, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let sub = wide(subkey);
    let mut key = HKEY::default();
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), None, access, &mut key) };
    (rc == ERROR_SUCCESS).then_some(key)
}

#[cfg(windows)]
fn read_in(subkey: &str, name: &str) -> Option<String> {
    let key = open(subkey, KEY_QUERY_VALUE)?;
    let name_w = wide(name);
    let mut size = 0u32;
    let probe = unsafe {
        RegQueryValueExW(key, PCWSTR(name_w.as_ptr()), None, None, None, Some(&mut size))
    };
    let mut out = None;
    if probe == ERROR_SUCCESS && size >= 2 {
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let rc = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name_w.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            )
        };
        if rc == ERROR_SUCCESS {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            out = Some(String::from_utf16_lossy(&buf[..len]));
        }
    }
    unsafe {
        let _ = RegCloseKey(key);
    }
    out
}

#[cfg(windows)]
fn write_in(subkey: &str, name: &str, value: &str) -> Result<(), String> {
    let key = open(subkey, KEY_SET_VALUE).ok_or("cannot open the registry key")?;
    let name_w = wide(name);
    let data = wide(value);
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    let rc = unsafe { RegSetValueExW(key, PCWSTR(name_w.as_ptr()), None, REG_SZ, Some(bytes)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if rc == ERROR_SUCCESS { Ok(()) } else { Err(format!("registry write failed ({})", rc.0)) }
}

#[cfg(windows)]
fn delete_in(subkey: &str, name: &str) -> Result<(), String> {
    let Some(key) = open(subkey, KEY_SET_VALUE) else { return Ok(()) };
    let name_w = wide(name);
    let rc = unsafe { RegDeleteValueW(key, PCWSTR(name_w.as_ptr())) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    // A value that is already gone is the state we wanted.
    if rc == ERROR_SUCCESS || rc.0 == 2 { Ok(()) } else { Err(format!("registry delete failed ({})", rc.0)) }
}

#[cfg(windows)]
pub fn is_enabled(identifier: &str) -> bool {
    read_in(RUN_KEY, &value_name(identifier)).is_some()
}

/// Enable (for the running exe) or disable. Returns the state afterwards.
#[cfg(windows)]
pub fn set_enabled(identifier: &str, enabled: bool) -> Result<bool, String> {
    let name = value_name(identifier);
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        write_in(RUN_KEY, &name, &run_value(&exe))?;
    } else {
        delete_in(RUN_KEY, &name)?;
    }
    Ok(is_enabled(identifier))
}

// ------------------------------------------------------------------ macOS / Linux

/// The agent label / the entry name: "com.macik.pulsetray" / "pulsetray" for the released app. A build with another
/// identifier (a test build) gets its own, so switching it on never repoints the installed copy.
#[cfg_attr(windows, allow(dead_code))]
fn unix_names(identifier: &str) -> (String, String) {
    if identifier == RELEASE_ID {
        ("com.macik.pulsetray".into(), "pulsetray".into())
    } else {
        (format!("com.macik.pulsetray.{identifier}"), format!("pulsetray-{identifier}"))
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The launch agent that starts the app at login (macOS): `<exe> --minimized`, once, when the user logs in.
#[cfg_attr(windows, allow(dead_code))]
pub fn launch_agent_plist(label: &str, exe: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{exe}</string>\n\
         \t\t<string>--minimized</string>\n\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         </dict>\n\
         </plist>\n",
        label = xml_escape(label),
        exe = xml_escape(&exe.display().to_string()),
    )
}

/// One argument of an XDG `Exec=` line: quoted when it has anything but plain characters, with the characters the
/// Desktop Entry spec names (`"`, `` ` ``, `$`, `\`) escaped by a backslash.
#[cfg_attr(windows, allow(dead_code))]
fn desktop_exec_arg(arg: &str) -> String {
    let plain = !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:,@=".contains(c));
    if plain {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The autostart entry (Linux): `Exec=<exe> --minimized`.
#[cfg_attr(windows, allow(dead_code))]
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=PulseTray\nComment=Lightweight system monitor for the tray\nExec={} --minimized\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_arg(&exe.display().to_string())
    )
}

/// `<home>/Library/LaunchAgents/<label>.plist`
#[cfg_attr(windows, allow(dead_code))]
pub fn launch_agent_path(home: &Path, label: &str) -> std::path::PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

/// `<config home>/autostart/<name>.desktop`; the config home is `$XDG_CONFIG_HOME` (when it is an absolute path), else `<home>/.config`.
#[cfg_attr(windows, allow(dead_code))]
pub fn autostart_entry_path(xdg_config_home: Option<&Path>, home: &Path, name: &str) -> std::path::PathBuf {
    let base = match xdg_config_home {
        Some(p) if p.is_absolute() => p.to_path_buf(),
        _ => home.join(".config"),
    };
    base.join("autostart").join(format!("{name}.desktop"))
}

/// Where this system keeps the autostart file of the app, and what goes into it (the running exe is started).
#[cfg(not(windows))]
fn unix_entry(identifier: &str) -> Result<(std::path::PathBuf, String), String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).ok_or("no home directory")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (label, name) = unix_names(identifier);
    if cfg!(target_os = "macos") {
        Ok((launch_agent_path(&home, &label), launch_agent_plist(&label, &exe)))
    } else {
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
        Ok((autostart_entry_path(xdg.as_deref(), &home, &name), desktop_entry(&exe)))
    }
}

#[cfg(not(windows))]
pub fn is_enabled(identifier: &str) -> bool {
    unix_entry(identifier).is_ok_and(|(path, _)| path.is_file())
}

/// Enable (for the running exe) or disable. Returns the state afterwards.
#[cfg(not(windows))]
pub fn set_enabled(identifier: &str, enabled: bool) -> Result<bool, String> {
    let (path, text) = unix_entry(identifier)?;
    if enabled {
        std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    } else if let Err(e) = std::fs::remove_file(&path) {
        // A file that is already gone is the state we wanted.
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.to_string());
        }
    }
    Ok(is_enabled(identifier))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use windows::Win32::System::Registry::{RegCreateKeyExW, RegDeleteKeyW, REG_OPTION_NON_VOLATILE};

    #[cfg(windows)]
    #[test]
    fn only_the_released_app_uses_the_plain_name() {
        assert_eq!(value_name("com.pulsetray.app"), "PulseTray");
        assert_eq!(value_name("com.pulsetray.menutest"), "PulseTray (com.pulsetray.menutest)");
    }

    #[cfg(windows)]
    #[test]
    fn value_is_quoted_and_minimized() {
        let v = run_value(Path::new("C:\\Users\\Ali Veli\\AppData\\Local\\PulseTray\\pulsetray.exe"));
        assert_eq!(v, "\"C:\\Users\\Ali Veli\\AppData\\Local\\PulseTray\\pulsetray.exe\" --minimized");
    }

    /// Exercises the real registry code against a throw-away key, never against the real Run key.
    #[cfg(windows)]
    #[test]
    fn write_read_delete_roundtrip() {
        let sub = format!("Software\\PulseTray_test_{}", std::process::id());
        let sub_w = wide(&sub);
        let mut key = HKEY::default();
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub_w.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                None,
                &mut key,
                None,
            )
        };
        assert_eq!(rc, ERROR_SUCCESS);
        unsafe {
            let _ = RegCloseKey(key);
        }

        let value = "\"C:\\Program Files\\x y\\pulsetray.exe\" --minimized";
        assert_eq!(read_in(&sub, "PulseTrayTest"), None);
        write_in(&sub, "PulseTrayTest", value).unwrap();
        assert_eq!(read_in(&sub, "PulseTrayTest").as_deref(), Some(value));
        delete_in(&sub, "PulseTrayTest").unwrap();
        assert_eq!(read_in(&sub, "PulseTrayTest"), None);
        delete_in(&sub, "PulseTrayTest").unwrap(); // deleting twice is fine

        unsafe {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(sub_w.as_ptr()));
        }
    }

    #[test]
    fn unix_names_keep_a_test_build_apart_from_the_installed_app() {
        assert_eq!(unix_names("com.pulsetray.app"), ("com.macik.pulsetray".to_string(), "pulsetray".to_string()));
        assert_eq!(
            unix_names("com.pulsetray.menutest"),
            ("com.macik.pulsetray.com.pulsetray.menutest".to_string(), "pulsetray-com.pulsetray.menutest".to_string())
        );
    }

    #[test]
    fn the_launch_agent_runs_the_exe_minimized_at_login() {
        let p = launch_agent_plist("com.macik.pulsetray", Path::new("/Applications/Pulse & Tray.app/Contents/MacOS/pulsetray"));
        assert!(p.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{p}");
        assert!(p.contains("<key>Label</key>\n\t<string>com.macik.pulsetray</string>"), "{p}");
        assert!(
            p.contains("<array>\n\t\t<string>/Applications/Pulse &amp; Tray.app/Contents/MacOS/pulsetray</string>\n\t\t<string>--minimized</string>\n\t</array>"),
            "exe escaped, then the flag: {p}"
        );
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"), "{p}");
        assert!(p.trim_end().ends_with("</plist>"), "{p}");
    }

    #[test]
    fn the_desktop_entry_quotes_a_path_with_spaces() {
        let plain = desktop_entry(Path::new("/opt/pulsetray/pulsetray"));
        assert!(plain.contains("\nExec=/opt/pulsetray/pulsetray --minimized\n"), "{plain}");
        assert!(plain.starts_with("[Desktop Entry]\nType=Application\nName=PulseTray\n"), "{plain}");
        let spaced = desktop_entry(Path::new("/home/ali veli/My $Apps/pulsetray"));
        assert!(spaced.contains("\nExec=\"/home/ali veli/My \\$Apps/pulsetray\" --minimized\n"), "{spaced}");
    }

    #[test]
    fn autostart_files_live_where_the_system_looks_for_them() {
        let home = Path::new("/home/ali");
        assert_eq!(launch_agent_path(home, "com.macik.pulsetray"), home.join("Library/LaunchAgents/com.macik.pulsetray.plist"));
        assert_eq!(autostart_entry_path(None, home, "pulsetray"), home.join(".config/autostart/pulsetray.desktop"));
        // `$XDG_CONFIG_HOME` is used when it is an absolute path, and ignored otherwise (as the spec says)
        let abs = std::env::current_dir().unwrap();
        assert_eq!(autostart_entry_path(Some(&abs), home, "pulsetray"), abs.join("autostart/pulsetray.desktop"));
        assert_eq!(autostart_entry_path(Some(Path::new("relative/dir")), home, "pulsetray"), home.join(".config/autostart/pulsetray.desktop"));
    }
}
