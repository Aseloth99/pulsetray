//! Self-update against macik.com.tr (same protocol as FlexiKey / MediaCompressor / Lumina):
//!
//!   GET /api/updates/check?appName=PulseTray&currentVersion=x.y.z
//!   -> { updateAvailable, version, download_url, release_notes, isMandatory, fileSize, checksum, notice }
//!
//! Three separate steps: `check` only asks (it returns an `Offer` for the UI), `download` fetches the package in the
//! background (progress, cancellable) and `install_and_relaunch` swaps the running exe for the new one. The package is
//! recognised by its content, not by its name (the download address has no extension, e.g.
//! /Pages/AppDownload?slug=pulsetray&file=PulseTray.exe): `MZ` = the exe itself, `PK` = a zip that holds `pulsetray.exe`.
//! Nothing is installed unless the download came from an allowed https host and matches the server's SHA-256 (when the
//! server sent one: an install the user asked for may go without, an automatic one never).

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const CHECK_URL: &str = "https://macik.com.tr/api/updates/check";
pub const APP_ID: &str = "PulseTray";
const ALLOWED_HOSTS: [&str; 1] = ["macik.com.tr"];
const EXE_IN_ZIP: &str = "pulsetray.exe";
/// The downloaded file (exe or zip) and the exe inside it.
const MAX_PKG: u64 = 300 * 1024 * 1024;
const MAX_EXE: u64 = 300 * 1024 * 1024;
/// Release notes are shown in a small box; a server that sends a book gets cut.
const MAX_NOTES: usize = 20_000;
const MAX_REDIRECTS: usize = 3;

// ------------------------------------------------------------------ errors

/// Everything that can go wrong between "the user pressed Update" and "the new exe is in place". The UI gets
/// `ui()` (`code` or `code: detail`) and puts the code into a sentence of the chosen language.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdErr {
    /// The user stopped the download.
    Cancelled,
    /// The file is not what the server announced (SHA-256).
    Checksum,
    /// No SHA-256 from the server and the size is not the announced one.
    Size,
    TooLarge,
    /// The download is neither an exe nor a zip with the exe in it.
    Package(String),
    /// The package is built for another CPU architecture.
    Arch,
    /// The folder of the exe can't be written to (read-only drive, Program Files, ...).
    NotWritable(String),
    /// A download address outside macik.com.tr.
    Url,
    Http(u16),
    Network(String),
    Io(String),
}

impl UpdErr {
    pub fn ui(&self) -> String {
        match self {
            UpdErr::Cancelled => "cancelled".into(),
            UpdErr::Checksum => "checksum".into(),
            UpdErr::Size => "size".into(),
            UpdErr::TooLarge => "too_large".into(),
            UpdErr::Package(d) => format!("package: {d}"),
            UpdErr::Arch => "arch".into(),
            UpdErr::NotWritable(d) => format!("not_writable: {d}"),
            UpdErr::Url => "url".into(),
            UpdErr::Http(c) => format!("http: {c}"),
            UpdErr::Network(d) => format!("network: {d}"),
            UpdErr::Io(d) => format!("io: {d}"),
        }
    }
}

impl std::fmt::Display for UpdErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdErr::Cancelled => write!(f, "cancelled"),
            UpdErr::Checksum => write!(f, "checksum mismatch"),
            UpdErr::Size => write!(f, "size mismatch"),
            UpdErr::TooLarge => write!(f, "download too large"),
            UpdErr::Package(d) => write!(f, "bad package: {d}"),
            UpdErr::Arch => write!(f, "update is built for another CPU architecture"),
            UpdErr::NotWritable(d) => write!(f, "cannot write next to the app: {d}"),
            UpdErr::Url => write!(f, "untrusted url"),
            UpdErr::Http(c) => write!(f, "http {c}"),
            UpdErr::Network(d) => write!(f, "{d}"),
            UpdErr::Io(d) => write!(f, "{d}"),
        }
    }
}

impl From<std::io::Error> for UpdErr {
    fn from(e: std::io::Error) -> Self {
        UpdErr::Io(e.to_string())
    }
}

// ------------------------------------------------------------------ the server's answer

/// What the server offers (what the UI shows on the update screen). The address and the checksum stay in Rust.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub version: String,
    /// Release notes as the server wrote them (HTML or plain text); the UI turns them into plain text.
    pub notes: String,
    /// Download size in bytes, when the server said.
    pub size: Option<u64>,
    #[serde(skip)]
    pub url: String,
    /// SHA-256 of the file (64 lower-case hex digits). None = the server sent none (or garbage).
    #[serde(skip)]
    pub sha256: Option<String>,
    pub mandatory: bool,
}

/// A package that is on disk (`%TEMP%\PulseTray_Update.pkg`) and was checked while it came in.
#[derive(Debug, Clone, PartialEq)]
pub struct Downloaded {
    pub version: String,
    pub path: PathBuf,
    /// SHA-256 of the file as downloaded; checked again right before installing (it may wait for hours).
    pub sha256: String,
}

#[derive(Debug)]
struct Reply {
    available: bool,
    version: Option<String>,
    url: Option<String>,
    notes: String,
    size: Option<u64>,
    sha256: Option<String>,
    mandatory: bool,
    /// Optional announcement (validated by `remote_notice::parse_notice`).
    notice: Option<Value>,
}

/// First of the names that is present and not null.
fn field<'a>(v: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|n| v.get(*n)).filter(|x| !x.is_null())
}

/// `true`, "true", "1", "yes", 1 ... anything else (also a missing / null / odd value) is false.
fn as_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "evet"),
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => false,
    }
}

fn as_text(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

/// A byte count that may come as a number, a decimal number or a string; 0, negative or garbage = unknown.
fn as_size(v: &Value) -> Option<u64> {
    let float = |f: f64| (f.is_finite() && f >= 1.0).then_some(f as u64);
    let n = match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().and_then(float)),
        Value::String(s) => {
            let t = s.trim();
            t.parse::<u64>().ok().or_else(|| t.parse::<f64>().ok().and_then(float))
        }
        _ => None,
    }?;
    (n > 0).then_some(n)
}

/// Release notes: a string, or a list of strings (one per line); anything else is no notes. Cut at `MAX_NOTES`.
fn as_notes(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    s.trim().chars().take(MAX_NOTES).collect()
}

/// A reply that is not a JSON object is not a reply at all (an error page, `null`, a list...).
fn parse_reply(v: &Value) -> Option<Reply> {
    if !v.is_object() {
        return None;
    }
    Some(Reply {
        available: field(v, &["updateAvailable", "update_available"]).is_some_and(as_bool),
        version: field(v, &["version"]).and_then(as_text),
        url: field(v, &["download_url", "downloadUrl"]).and_then(as_text),
        notes: field(v, &["release_notes", "releaseNotes"]).map(as_notes).unwrap_or_default(),
        size: field(v, &["fileSize", "file_size"]).and_then(as_size),
        sha256: field(v, &["checksum"]).and_then(Value::as_str).and_then(normalize_checksum),
        mandatory: field(v, &["isMandatory", "is_mandatory"]).is_some_and(as_bool),
        notice: v.get("notice").filter(|n| !n.is_null()).cloned(),
    })
}

/// The offer for this machine, or None (no update, not newer, no address, or a version that had to be rolled back).
/// The address is not judged here (`check` does, so the failure can be reported).
fn make_offer(reply: Reply, current: &str, skipped: Option<&str>) -> Option<Offer> {
    if !reply.available {
        return None;
    }
    let (version, url) = (reply.version?, reply.url?);
    let version = version.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, current) {
        return None;
    }
    // A version that already had to be rolled back (it kept crashing here) is not offered again.
    if skipped == Some(version.as_str()) {
        return None;
    }
    Some(Offer { version, notes: reply.notes, size: reply.size, url, sha256: reply.sha256, mandatory: reply.mandatory })
}

/// https only, host must be an allowed domain or one of its subdomains, no credentials / odd ports.
pub fn validate_url(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw.trim()) else { return false };
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return false;
    }
    if u.port().is_some_and(|p| p != 443) {
        return false;
    }
    let Some(host) = u.host_str().map(str::to_lowercase) else { return false };
    ALLOWED_HOSTS
        .iter()
        .any(|a| host == *a || host.ends_with(&format!(".{a}")))
}

/// Accepts `sha256:` prefix, spaces, upper case; anything that isn't 64 hex digits is None.
pub fn normalize_checksum(v: &str) -> Option<String> {
    let t = v.trim().to_lowercase();
    let t = t.strip_prefix("sha256:").unwrap_or(&t).trim().to_string();
    (t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())).then_some(t)
}

fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_version(latest), parse_version(current)) else {
        return false;
    };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

// ------------------------------------------------------------------ talking to the server

/// The check reply is a few hundred bytes; the download is a few MB and may come over a slow link.
const CHECK_BODY: Duration = Duration::from_secs(30);
const DOWNLOAD_BODY: Duration = Duration::from_secs(20 * 60);

/// `body` = time allowed for receiving the response body (connecting and the headers have their own limits).
fn agent(body: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(body))
        // Redirects are followed by hand so every hop can be validated.
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent(format!("AppUpp/1.0 {APP_ID}-UpdateChecker"))
        .build()
        .into()
}

/// GET with manual, validated redirects.
fn get(agent: &ureq::Agent, start: &str) -> Result<ureq::http::Response<ureq::Body>, UpdErr> {
    let mut url = start.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !validate_url(&url) {
            return Err(UpdErr::Url);
        }
        let resp = agent.get(&url).call().map_err(|e| UpdErr::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let loc = resp
                .headers()
                .get("location")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| UpdErr::Network("redirect without location".into()))?;
            url = url::Url::parse(&url)
                .and_then(|base| base.join(loc))
                .map_err(|e| UpdErr::Network(e.to_string()))?
                .to_string();
            continue;
        }
        if status != 200 {
            return Err(UpdErr::Http(status));
        }
        return Ok(resp);
    }
    Err(UpdErr::Network("too many redirects".into()))
}

/// What the server said: the offer for this machine (if there is a newer version) and the raw announcement.
pub struct Checked {
    pub offer: Option<Offer>,
    pub notice: Option<Value>,
}

/// Ask the server. The request carries the app name and the running version, nothing else. Only asks:
/// nothing is downloaded.
pub fn check(current: &str) -> Result<Checked, String> {
    let agent = agent(CHECK_BODY);
    let url = format!("{CHECK_URL}?appName={APP_ID}&currentVersion={current}");
    let body: Value = get(&agent, &url)
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;
    let mut reply = parse_reply(&body).ok_or("the update server sent an unexpected reply")?;
    let notice = reply.notice.take();
    let offer = make_offer(reply, current, skipped_version().as_deref());
    if let Some(o) = offer.as_ref().filter(|o| !validate_url(&o.url)) {
        return Err(format!("server offered an untrusted download url: {}", o.url));
    }
    Ok(Checked { offer, notice })
}

// ------------------------------------------------------------------ downloading

/// Lets an action through at most once per `every` (the download reports its progress to the UI ~5 times a second).
pub struct Throttle {
    every: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(every: Duration) -> Self {
        Self { every, last: None }
    }

    pub fn due(&mut self, now: Instant) -> bool {
        if self.last.is_none_or(|l| now.saturating_duration_since(l) >= self.every) {
            self.last = Some(now);
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PackageKind {
    Exe,
    Zip,
}

/// What a downloaded file is, from its first bytes: `MZ` = a Windows executable, `PK` = a zip.
pub fn sniff(head: &[u8]) -> Option<PackageKind> {
    match head {
        [b'M', b'Z', ..] => Some(PackageKind::Exe),
        [b'P', b'K', ..] => Some(PackageKind::Zip),
        _ => None,
    }
}

pub fn package_path() -> PathBuf {
    std::env::temp_dir().join(format!("{APP_ID}_Update.pkg"))
}

/// A package (or half of one) that an earlier run left in %TEMP% (the app was quit while it waited for a quiet moment
/// or while it was coming in) is no use to anybody: up to 300 MB must not stay there. Called once at start-up.
pub fn remove_stale_download() {
    remove_package_files(&package_path());
}

fn remove_package_files(pkg: &Path) {
    let _ = std::fs::remove_file(pkg);
    let _ = std::fs::remove_file(with_suffix(pkg, ".part"));
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

struct Limits<'a> {
    /// The server's SHA-256, if it sent one.
    sha256: Option<&'a str>,
    /// The announced size; only judged when there is no SHA-256 (a matching hash says it all).
    size: Option<u64>,
    /// What the UI gets as the total (Content-Length, else the announced size, else 0 = unknown).
    total_hint: u64,
    max: u64,
}

/// Copies `reader` into `part`, hashing on the way. Stops at once for a stop request, a file that is neither an exe
/// nor a zip (an error page, say: no point in downloading the rest), or more than `max` bytes. Returns the SHA-256.
fn stream_to_file(
    reader: &mut dyn Read,
    part: &Path,
    lim: &Limits,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<String, UpdErr> {
    let mut file = std::fs::File::create(part)?;
    let mut hasher = Sha256::new();
    let (mut buf, mut total) = (vec![0u8; 64 * 1024], 0u64);
    let mut head: Vec<u8> = Vec::with_capacity(2);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(UpdErr::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| UpdErr::Network(e.to_string()))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > lim.max {
            return Err(UpdErr::TooLarge);
        }
        if head.len() < 2 {
            head.extend_from_slice(&buf[..n.min(2 - head.len())]);
            if head.len() == 2 && sniff(&head).is_none() {
                return Err(UpdErr::Package("not an exe or zip".into()));
            }
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        progress(total, lim.total_hint);
    }
    file.flush()?;
    if sniff(&head).is_none() {
        return Err(UpdErr::Package(if total == 0 { "empty".into() } else { "not an exe or zip".into() }));
    }
    progress(total, total);
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    match (lim.sha256, lim.size) {
        (Some(expect), _) if actual != expect => Err(UpdErr::Checksum),
        (Some(_), _) => Ok(actual),
        (None, Some(size)) if size != total => Err(UpdErr::Size),
        (None, _) => Ok(actual),
    }
}

/// Fetches the offered package into `%TEMP%\PulseTray_Update.pkg` (written as `.part` first). `progress(done, total)`
/// is called for every chunk (the caller thins it out); `cancel` stops it. Total is 0 when nobody knows the size.
pub fn download(offer: &Offer, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<Downloaded, UpdErr> {
    if !validate_url(&offer.url) {
        return Err(UpdErr::Url);
    }
    if offer.size.is_some_and(|s| s > MAX_PKG) {
        return Err(UpdErr::TooLarge);
    }
    let pkg = package_path();
    let part = with_suffix(&pkg, ".part");
    let url = format!("{}{}_t={}", offer.url, if offer.url.contains('?') { "&" } else { "?" }, now_secs());
    let mut resp = get(&agent(DOWNLOAD_BODY), &url)?;
    let total_hint = resp.body().content_length().or(offer.size).unwrap_or(0);
    if total_hint > MAX_PKG {
        return Err(UpdErr::TooLarge);
    }
    let lim = Limits { sha256: offer.sha256.as_deref(), size: offer.size, total_hint, max: MAX_PKG };
    let sha = {
        let mut reader = resp.body_mut().as_reader();
        stream_to_file(&mut reader, &part, &lim, cancel, &mut progress)
    };
    let sha = match sha {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&pkg);
    if let Err(e) = std::fs::rename(&part, &pkg) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    Ok(Downloaded { version: offer.version.clone(), path: pkg, sha256: sha })
}

/// The folder of the exe must take the new exe (`.new` next to it): better to find out before downloading.
pub fn check_writable(exe: &Path) -> Result<(), UpdErr> {
    let dir = exe.parent().ok_or_else(|| UpdErr::NotWritable("no folder".into()))?;
    let probe = dir.join(format!("{APP_ID}.write-test"));
    let result = std::fs::File::create(&probe).and_then(|mut f| f.write_all(b"x"));
    let _ = std::fs::remove_file(&probe);
    result.map_err(|e| UpdErr::NotWritable(e.to_string()))
}

// ------------------------------------------------------------------ installing

/// The CPU architecture a Windows executable was built for (IMAGE_FILE_HEADER.Machine):
/// 0x8664 = x64, 0xAA64 = ARM64, 0x014C = x86. `None` when the bytes are not a PE file.
pub fn pe_machine(head: &[u8]) -> Option<u16> {
    if head.len() < 0x40 || &head[..2] != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(head[0x3C..0x40].try_into().ok()?) as usize;
    if head.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let m = head.get(pe + 4..pe + 6)?;
    Some(u16::from_le_bytes([m[0], m[1]]))
}

/// Pull `pulsetray.exe` (any folder inside the zip) out as bytes; must be a real Windows executable.
pub fn extract_exe<R: Read + std::io::Seek>(zip: R) -> Result<Vec<u8>, UpdErr> {
    let mut archive = zip::ZipArchive::new(zip).map_err(|e| UpdErr::Package(e.to_string()))?;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| UpdErr::Package(e.to_string()))?;
        let name = entry.name().map_err(|e| UpdErr::Package(e.to_string()))?.replace('\\', "/");
        if name.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(EXE_IN_ZIP)) {
            let mut data = Vec::new();
            entry.take(MAX_EXE + 1).read_to_end(&mut data)?;
            if data.len() as u64 > MAX_EXE {
                return Err(UpdErr::TooLarge);
            }
            if pe_machine(&data).is_none() {
                return Err(UpdErr::Package("the exe in the package is not a Windows executable".into()));
            }
            return Ok(data);
        }
    }
    Err(UpdErr::Package(format!("{EXE_IN_ZIP} not found in the package")))
}

/// The new exe out of a package, whichever kind it is (judged by the content).
pub fn exe_from_package(bytes: Vec<u8>) -> Result<Vec<u8>, UpdErr> {
    match sniff(&bytes) {
        Some(PackageKind::Exe) => {
            if bytes.len() as u64 > MAX_EXE {
                return Err(UpdErr::TooLarge);
            }
            if pe_machine(&bytes).is_none() {
                return Err(UpdErr::Package("the file is not a Windows executable".into()));
            }
            Ok(bytes)
        }
        Some(PackageKind::Zip) => extract_exe(std::io::Cursor::new(bytes)),
        None => Err(UpdErr::Package("not an exe or zip".into())),
    }
}

/// A running exe can be renamed on Windows, so: write `.new`, move current to `.old`, move `.new` in.
/// Rolls back if the final rename fails.
pub fn swap_exe(current: &Path, data: &[u8]) -> Result<(), UpdErr> {
    let new = with_suffix(current, ".new");
    let old = with_suffix(current, ".old");
    std::fs::write(&new, data).map_err(|e| UpdErr::NotWritable(e.to_string()))?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(current, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(e.into());
    }
    if let Err(e) = std::fs::rename(&new, current) {
        // Put the old one back; if even that fails, keep `.new` too, so at least one copy survives.
        if std::fs::rename(&old, current).is_ok() {
            let _ = std::fs::remove_file(&new);
        }
        return Err(e.into());
    }
    Ok(())
}

/// After an update the previous version is kept as `.old` until the new one has run for a while
/// (`confirm_healthy`). Starts in between are counted; if the new version dies before that twice, the previous
/// version comes back and the broken one is remembered so it isn't installed again. (Release builds abort on a
/// panic, so a version that crashes at start-up would otherwise just vanish from the tray on every PC.)
#[derive(Debug, PartialEq)]
pub enum Guard {
    /// Not the first starts after an update.
    Normal,
    /// Start number `n` since the update, still on probation.
    Probation(u32),
    /// The previous version was restored; the caller starts it and exits.
    RolledBack,
}

/// Starts before the new version counts as broken (the first one is the restart right after installing).
const MAX_PROBATION_STARTS: u32 = 2;

pub fn startup_guard() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    if guard_at(&exe, env!("CARGO_PKG_VERSION")) != Guard::RolledBack {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let _ = std::process::Command::new(&exe).arg("--minimized").creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS).spawn();
    }
    true
}

fn guard_at(exe: &Path, version: &str) -> Guard {
    let old = with_suffix(exe, ".old");
    if !old.exists() {
        return Guard::Normal;
    }
    let boot = with_suffix(exe, ".boot");
    let starts = std::fs::read_to_string(&boot).ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0) + 1;
    if starts <= MAX_PROBATION_STARTS {
        let _ = std::fs::write(&boot, starts.to_string());
        return Guard::Probation(starts);
    }
    // This version kept dying: the running exe can be renamed, so swap the previous one back in.
    let bad = with_suffix(exe, ".bad");
    let _ = std::fs::remove_file(&bad);
    if std::fs::rename(exe, &bad).is_err() {
        return Guard::Probation(starts);
    }
    if std::fs::rename(&old, exe).is_err() {
        let _ = std::fs::rename(&bad, exe);
        return Guard::Probation(starts);
    }
    let _ = std::fs::write(with_suffix(exe, ".skip"), version);
    let _ = std::fs::remove_file(&boot);
    Guard::RolledBack
}

/// The version that had to be rolled back on this PC (never offered again).
fn skipped_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let v = std::fs::read_to_string(with_suffix(&exe, ".skip")).ok()?;
    Some(v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The running version has proven itself: drop what the last update left behind.
pub fn confirm_healthy() {
    if let Ok(exe) = std::env::current_exe() {
        clean_at(&exe);
    }
}

/// A clean exit also proves the version (the user quit it after it had been up for a while) — but not the exit that
/// installs an update: by then the exe on disk is already the NEW one and its `.old` / `.boot` must stay.
pub fn healthy_on_exit(uptime: Duration, exiting_for_update: bool) -> bool {
    !exiting_for_update && uptime >= Duration::from_secs(10)
}

fn clean_at(exe: &Path) {
    for suffix in [".old", ".new", ".boot", ".bad"] {
        let _ = std::fs::remove_file(with_suffix(exe, suffix));
    }
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ------------------------------------------------------------------ command line

/// The value after `flag` (`--wait-pid 123`).
pub fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// The version a fresh update came from (`--updated 1.0.0`): digits and dots only.
pub fn updated_from(args: &[String]) -> Option<String> {
    let v = arg_value(args, "--updated")?;
    let v = v.trim().trim_start_matches('v');
    let ok = !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    ok.then(|| v.to_string())
}

/// How the new exe is started: quietly (a tray app: the popup stays closed), after our process (`--wait-pid`,
/// otherwise the single-instance guard would hand its launch to us and swallow it), and told what it was updated from.
pub fn relaunch_args(pid: u32, from_version: &str) -> Vec<String> {
    vec!["--minimized".into(), "--wait-pid".into(), pid.to_string(), "--updated".into(), from_version.into()]
}

/// Puts the verified package in place of `exe` (the running one). On any error `exe` is still the old, working one.
fn install_into(exe: &Path, pkg: &Downloaded) -> Result<(), UpdErr> {
    // The package may have waited in %TEMP% for hours: check it is still exactly what came in.
    let mut bytes = Vec::new();
    std::fs::File::open(&pkg.path)?.take(MAX_PKG + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PKG {
        return Err(UpdErr::TooLarge);
    }
    if sha_hex(&bytes) != pkg.sha256 {
        let _ = std::fs::remove_file(&pkg.path);
        return Err(UpdErr::Checksum);
    }
    let data = exe_from_package(bytes)?;

    // A package for another CPU architecture would leave every install without a working exe.
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(exe).and_then(|mut f| f.read(&mut head))?;
    if let (Some(mine), Some(theirs)) = (pe_machine(&head[..n]), pe_machine(&data)) {
        if mine != theirs {
            return Err(UpdErr::Arch);
        }
    }

    swap_exe(exe, &data)?;
    let _ = std::fs::remove_file(&pkg.path);
    Ok(())
}

/// Install the verified package over the running exe, then start the new one (see `relaunch_args`).
pub fn install_and_relaunch(pkg: &Downloaded, from_version: &str) -> Result<(), UpdErr> {
    let exe = std::env::current_exe()?;
    install_into(&exe, pkg)?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        std::process::Command::new(&exe)
            .args(relaunch_args(std::process::id(), from_version))
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn()?;
    }
    #[cfg(not(windows))]
    let _ = from_version;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_new_version_that_keeps_dying_is_rolled_back_and_skipped() {
        let dir = std::env::temp_dir().join(format!("pt_guard_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("pulsetray.exe");
        std::fs::write(&exe, b"new").unwrap();
        // No update happened: nothing to do.
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::Normal);
        std::fs::write(with_suffix(&exe, ".old"), b"old").unwrap();
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::Probation(1), "restart right after installing");
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::Probation(2), "it died once; the user starts it again");
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::RolledBack, "died again: the previous version comes back");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert_eq!(std::fs::read(with_suffix(&exe, ".bad")).unwrap(), b"new");
        assert_eq!(std::fs::read_to_string(with_suffix(&exe, ".skip")).unwrap(), "2.0.0");
        assert!(!with_suffix(&exe, ".old").exists() && !with_suffix(&exe, ".boot").exists());
        assert_eq!(guard_at(&exe, "1.0.0"), Guard::Normal, "the restored version runs normally");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_healthy_new_version_cleans_up_and_is_not_on_probation_any_more() {
        let dir = std::env::temp_dir().join(format!("pt_guard_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("pulsetray.exe");
        std::fs::write(&exe, b"new").unwrap();
        std::fs::write(with_suffix(&exe, ".old"), b"old").unwrap();
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::Probation(1));
        clean_at(&exe);
        assert_eq!(guard_at(&exe, "2.0.0"), Guard::Normal);
        assert!(!with_suffix(&exe, ".old").exists() && !with_suffix(&exe, ".boot").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_exit_confirms_the_version_unless_it_is_the_update_exit_or_too_early() {
        assert!(healthy_on_exit(Duration::from_secs(10), false));
        assert!(healthy_on_exit(Duration::from_secs(3600), false));
        assert!(!healthy_on_exit(Duration::from_secs(9), false), "died young: no proof yet");
        assert!(!healthy_on_exit(Duration::from_secs(3600), true), "the exit that installs an update must keep .old / .boot");
    }

    #[test]
    fn url_policy() {
        for ok in [
            "https://macik.com.tr/d/PulseTray.zip",
            "https://cdn.macik.com.tr/a.zip",
            "https://macik.com.tr:443/a.zip",
            // the real download address has no extension
            "https://macik.com.tr/Pages/AppDownload?slug=pulsetray&file=PulseTray.exe",
        ] {
            assert!(validate_url(ok), "{ok}");
        }
        for bad in [
            "http://macik.com.tr/a.zip",
            "https://macik.com.tr.evil.example/a.zip",
            "https://evilmacik.com.tr/a.zip",
            "https://user:pw@macik.com.tr/a.zip",
            "https://macik.com.tr@evil.com/a.zip",
            "https://macik.com.tr:8443/a.zip",
            "ftp://macik.com.tr/a.zip",
            "file:///C:/a.zip",
            "macik.com.tr/a.zip",
            "",
        ] {
            assert!(!validate_url(bad), "{bad}");
        }
    }

    #[test]
    fn checksum_and_versions() {
        let h = "A".repeat(64);
        assert_eq!(normalize_checksum(&format!(" sha256:{h} ")), Some("a".repeat(64)));
        assert_eq!(normalize_checksum("abc"), None);
        assert_eq!(normalize_checksum(&"g".repeat(64)), None);
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    fn make_zip(name: &str, data: &[u8]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    /// Smallest byte string that `pe_machine` accepts: DOS header, e_lfanew = 0x80, "PE\0\0", Machine.
    fn fake_pe(machine: u16, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 0x80];
        v[..2].copy_from_slice(b"MZ");
        v[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        v.extend_from_slice(b"PE\0\0");
        v.extend_from_slice(&machine.to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn extracts_exe_from_nested_folder() {
        let exe = fake_pe(0x8664, b"-new-version");
        let z = make_zip("PulseTray/PulseTray.exe", &exe);
        assert_eq!(extract_exe(Cursor::new(z)).unwrap(), exe);
        assert!(extract_exe(Cursor::new(make_zip("readme.txt", &exe))).is_err());
        assert!(extract_exe(Cursor::new(make_zip("pulsetray.exe", b"not an exe"))).is_err());
        // "MZ" alone is not enough any more: a truncated / fake file must be rejected
        assert!(extract_exe(Cursor::new(make_zip("pulsetray.exe", b"MZ-fake"))).is_err());
    }

    #[test]
    fn pe_machine_reads_the_architecture() {
        assert_eq!(pe_machine(&fake_pe(0x8664, b"")), Some(0x8664));
        assert_eq!(pe_machine(&fake_pe(0xAA64, b"x")), Some(0xAA64));
        assert_eq!(pe_machine(b"MZ"), None);
        assert_eq!(pe_machine(&[0u8; 200]), None);
        let mut bad_sig = fake_pe(0x8664, b"");
        bad_sig[0x80] = b'X';
        assert_eq!(pe_machine(&bad_sig), None);
        // e_lfanew pointing outside the buffer must not panic
        let mut far = fake_pe(0x8664, b"");
        far[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(pe_machine(&far), None);
        // the running test binary itself is a real PE file for this architecture
        let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        assert!(pe_machine(&me[..4096.min(me.len())]).is_some());
    }

    /// The updater compares Cargo's version, the UI and the release file names use tauri.conf.json and
    /// package.json. If they ever disagree the same build would be "updated" to itself again and again.
    #[test]
    fn all_three_version_numbers_agree() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let pkg: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
        assert_eq!(conf["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(pkg["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn swap_replaces_and_keeps_old() {
        let dir = std::env::temp_dir().join(format!("pt_swap_{}", now_secs()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("pulsetray.exe");
        std::fs::write(&exe, b"MZ-old").unwrap();
        swap_exe(&exe, b"MZ-new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ-new");
        assert_eq!(std::fs::read(with_suffix(&exe, ".old")).unwrap(), b"MZ-old");
        assert!(!with_suffix(&exe, ".new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- the server's answer

    fn offer_json() -> Value {
        json!({
            "updateAvailable": true,
            "version": "1.2.0",
            "download_url": "https://macik.com.tr/Pages/AppDownload?slug=pulsetray&file=PulseTray.exe",
            "release_notes": "<p>Yeni</p>",
            "isMandatory": false,
            "fileSize": 7_654_321u64,
            "checksum": "A".repeat(64),
        })
    }

    fn offer_of(v: &Value, current: &str) -> Option<Offer> {
        make_offer(parse_reply(v).unwrap(), current, None)
    }

    #[test]
    fn a_complete_reply_becomes_an_offer() {
        let o = offer_of(&offer_json(), "1.0.0").unwrap();
        assert_eq!(o.version, "1.2.0");
        assert_eq!(o.notes, "<p>Yeni</p>");
        assert_eq!(o.size, Some(7_654_321));
        assert_eq!(o.sha256, Some("a".repeat(64)));
        assert!(!o.mandatory);
        assert!(o.url.ends_with("file=PulseTray.exe"));
        // what the UI gets: no address, no checksum
        let ui = serde_json::to_value(&o).unwrap();
        assert_eq!(ui, json!({"version": "1.2.0", "notes": "<p>Yeni</p>", "size": 7_654_321u64, "mandatory": false}));
    }

    #[test]
    fn odd_field_types_do_not_break_the_reply() {
        // "false" as a string is false (it used to be a parse error or, worse, truthy)
        let mut v = offer_json();
        v["updateAvailable"] = json!("false");
        assert!(offer_of(&v, "1.0.0").is_none());
        v["updateAvailable"] = json!("True");
        assert!(offer_of(&v, "1.0.0").is_some());
        v["updateAvailable"] = json!(1);
        assert!(offer_of(&v, "1.0.0").is_some());

        let mut v = offer_json();
        v["isMandatory"] = json!("true");
        assert!(offer_of(&v, "1.0.0").unwrap().mandatory);
        v["isMandatory"] = json!([1]);
        assert!(!offer_of(&v, "1.0.0").unwrap().mandatory);

        // fileSize: string, decimal, null, negative, zero, text, huge, wrong type -> a number or unknown, never an error
        for (given, want) in [
            (json!("7654321"), Some(7_654_321)),
            (json!(" 12 "), Some(12)),
            (json!(1.5e6), Some(1_500_000)),
            (json!("2048.0"), Some(2048)),
            (json!(null), None),
            (json!(-5), None),
            (json!(0), None),
            (json!("big"), None),
            (json!(""), None),
            (json!({"a": 1}), None),
            (json!(true), None),
            (json!(u64::MAX), Some(u64::MAX)),
            (json!(1e300), Some(u64::MAX)),
        ] {
            let mut v = offer_json();
            v["fileSize"] = given.clone();
            assert_eq!(offer_of(&v, "1.0.0").unwrap().size, want, "fileSize {given}");
        }

        // checksum: null, empty, short, wrong type, not hex -> no checksum; prefix and case are fine
        for given in [json!(null), json!(""), json!("abc"), json!(12345), json!("z".repeat(64)), json!(["a"])] {
            let mut v = offer_json();
            v["checksum"] = given.clone();
            assert_eq!(offer_of(&v, "1.0.0").unwrap().sha256, None, "checksum {given}");
        }
        let mut v = offer_json();
        v["checksum"] = json!(format!("sha256:{}", "AB".repeat(32)));
        assert_eq!(offer_of(&v, "1.0.0").unwrap().sha256, Some("ab".repeat(32)));

        // fields missing altogether
        let o = offer_of(&json!({"updateAvailable": true, "version": "1.2.0", "download_url": "https://macik.com.tr/x"}), "1.0.0").unwrap();
        assert_eq!((o.notes.as_str(), o.size, o.sha256, o.mandatory), ("", None, None, false));
        // a numeric version, a v prefix
        let o = offer_of(&json!({"updateAvailable": true, "version": 1.5, "download_url": "https://macik.com.tr/x"}), "1.0.0").unwrap();
        assert_eq!(o.version, "1.5");
        let o = offer_of(&json!({"updateAvailable": true, "version": "v1.5.1", "download_url": "https://macik.com.tr/x"}), "1.0.0").unwrap();
        assert_eq!(o.version, "1.5.1");
    }

    #[test]
    fn notes_come_as_text_html_a_list_or_nothing() {
        let notes = |given: Value| {
            let mut v = offer_json();
            v["release_notes"] = given;
            offer_of(&v, "1.0.0").unwrap().notes
        };
        assert_eq!(notes(json!("  düz metin\nikinci satır ")), "düz metin\nikinci satır");
        assert_eq!(notes(json!("<ul><li>a</li><li>b &amp; c</li></ul>")), "<ul><li>a</li><li>b &amp; c</li></ul>");
        assert_eq!(notes(json!(["bir", "iki", 3])), "bir\niki");
        assert_eq!(notes(json!(null)), "");
        assert_eq!(notes(json!(42)), "");
        assert_eq!(notes(json!({"tr": "x"})), "");
        // cut at a char boundary
        let long = "ğ".repeat(MAX_NOTES + 500);
        assert_eq!(notes(json!(long)).chars().count(), MAX_NOTES);
    }

    #[test]
    fn who_gets_an_offer() {
        let v = offer_json();
        assert!(offer_of(&v, "1.0.0").is_some());
        assert!(offer_of(&v, "1.2.0").is_none(), "same version");
        assert!(offer_of(&v, "1.3.0").is_none(), "older than ours");

        let mut none = offer_json();
        none["updateAvailable"] = json!(false);
        assert!(offer_of(&none, "1.0.0").is_none());

        for missing in ["version", "download_url"] {
            let mut m = offer_json();
            m.as_object_mut().unwrap().remove(missing);
            assert!(offer_of(&m, "1.0.0").is_none(), "{missing}");
        }
        // a version that had to be rolled back on this PC is not offered again (`.skip`)
        assert!(make_offer(parse_reply(&v).unwrap(), "1.0.0", Some("1.2.0")).is_none());
        assert!(make_offer(parse_reply(&v).unwrap(), "1.0.0", Some("1.1.0")).is_some());
    }

    #[test]
    fn a_reply_that_is_not_an_object_is_refused_and_the_notice_travels_on() {
        for bad in [json!(null), json!([1, 2]), json!("<html>"), json!(7)] {
            assert!(parse_reply(&bad).is_none(), "{bad}");
        }
        let mut v = offer_json();
        v["notice"] = json!({"id": "n1", "text_tr": "Merhaba"});
        assert_eq!(parse_reply(&v).unwrap().notice, Some(json!({"id": "n1", "text_tr": "Merhaba"})));
        v["notice"] = json!(null);
        assert_eq!(parse_reply(&v).unwrap().notice, None);
        // an empty object: no update, but a valid reply (the device is registered)
        assert!(!parse_reply(&json!({})).unwrap().available);
    }

    // ---- the package

    #[test]
    fn the_package_kind_comes_from_the_content() {
        assert_eq!(sniff(b"MZ\x90\0"), Some(PackageKind::Exe));
        assert_eq!(sniff(b"PK\x03\x04"), Some(PackageKind::Zip));
        assert_eq!(sniff(b"PK"), Some(PackageKind::Zip));
        assert_eq!(sniff(b"<html>"), None);
        assert_eq!(sniff(b"M"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"Rar!\x1a\x07"), None);

        let exe = fake_pe(0x8664, b"-exe");
        assert_eq!(exe_from_package(exe.clone()).unwrap(), exe, "the exe itself");
        assert_eq!(exe_from_package(make_zip("PulseTray/pulsetray.exe", &exe)).unwrap(), exe, "a zip with the exe in it");
        assert!(matches!(exe_from_package(b"MZ-not-really".to_vec()), Err(UpdErr::Package(_))));
        assert!(matches!(exe_from_package(make_zip("other.exe", &exe)), Err(UpdErr::Package(_))));
        assert!(matches!(exe_from_package(b"<html>404</html>".to_vec()), Err(UpdErr::Package(_))));
        assert!(matches!(exe_from_package(Vec::new()), Err(UpdErr::Package(_))));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pt_upd_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A reader that hands out `data` in pieces of `step` bytes (like a network body).
    struct Pieces<'a> {
        data: &'a [u8],
        step: usize,
        on_read: Option<Box<dyn FnMut(usize) + 'a>>,
        reads: usize,
    }
    impl Read for Pieces<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if let Some(f) = self.on_read.as_mut() {
                f(self.reads);
            }
            let n = self.step.min(self.data.len()).min(out.len());
            out[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }
    fn pieces(data: &[u8], step: usize) -> Pieces<'_> {
        Pieces { data, step, on_read: None, reads: 0 }
    }

    fn run_stream(data: &[u8], lim: &Limits, cancel: &AtomicBool, dir: &Path) -> (Result<String, UpdErr>, Vec<(u64, u64)>) {
        let mut seen = Vec::new();
        let part = dir.join("x.pkg.part");
        let r = stream_to_file(&mut pieces(data, 100_000), &part, lim, cancel, &mut |d, t| seen.push((d, t)));
        (r, seen)
    }

    #[test]
    fn a_download_is_hashed_counted_and_written() {
        let dir = temp("stream_ok");
        let data = fake_pe(0x8664, &vec![7u8; 250_000]);
        let want = sha_hex(&data);
        let lim = Limits { sha256: Some(want.as_str()), size: Some(data.len() as u64), total_hint: data.len() as u64, max: MAX_PKG };
        let (r, seen) = run_stream(&data, &lim, &AtomicBool::new(false), &dir);
        assert_eq!(r.unwrap(), want);
        assert_eq!(std::fs::read(dir.join("x.pkg.part")).unwrap(), data, "what came in is on disk");
        let total = data.len() as u64;
        assert!(seen.len() >= 3, "progress per chunk: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "never goes back");
        assert_eq!(*seen.last().unwrap(), (total, total), "ends at 100%");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_wrong_checksum_is_refused_but_a_missing_one_is_not() {
        let dir = temp("stream_sha");
        let data = fake_pe(0x8664, b"payload");
        let ok = AtomicBool::new(false);
        let wrong = "0".repeat(64);
        let lim = Limits { sha256: Some(wrong.as_str()), size: None, total_hint: 0, max: MAX_PKG };
        assert_eq!(run_stream(&data, &lim, &ok, &dir).0, Err(UpdErr::Checksum));
        // no checksum from the server: fine for an install the user asked for (the caller decides about automatic ones)
        let lim = Limits { sha256: None, size: None, total_hint: 0, max: MAX_PKG };
        assert_eq!(run_stream(&data, &lim, &ok, &dir).0.unwrap(), sha_hex(&data));
        // ... and then the announced size is what is left to check
        let lim = Limits { sha256: None, size: Some(data.len() as u64 + 1), total_hint: 0, max: MAX_PKG };
        assert_eq!(run_stream(&data, &lim, &ok, &dir).0, Err(UpdErr::Size));
        let lim = Limits { sha256: None, size: Some(data.len() as u64), total_hint: 0, max: MAX_PKG };
        assert!(run_stream(&data, &lim, &ok, &dir).0.is_ok());
        // a matching hash makes a stale size harmless
        let good = sha_hex(&data);
        let lim = Limits { sha256: Some(good.as_str()), size: Some(1), total_hint: 0, max: MAX_PKG };
        assert!(run_stream(&data, &lim, &ok, &dir).0.is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_is_not_an_exe_or_a_zip_stops_at_the_first_bytes() {
        let dir = temp("stream_kind");
        let ok = AtomicBool::new(false);
        let lim = Limits { sha256: None, size: None, total_hint: 0, max: MAX_PKG };
        // an HTML error page with status 200, delivered in tiny pieces: refused after the first piece
        let page = b"<html>not found</html>".repeat(10_000);
        let mut r = pieces(&page, 5);
        let part = dir.join("p.part");
        let got = stream_to_file(&mut r, &part, &lim, &ok, &mut |_, _| {});
        assert!(matches!(got, Err(UpdErr::Package(_))), "{got:?}");
        assert_eq!(r.reads, 1, "the rest was not downloaded");
        // a rar, an empty body, a single byte
        for data in [&b"Rar!\x1a\x07\x00rest"[..], &b""[..], &b"M"[..]] {
            assert!(matches!(run_stream(data, &lim, &ok, &dir).0, Err(UpdErr::Package(_))), "{data:?}");
        }
        // both real kinds pass
        assert!(run_stream(&fake_pe(0x8664, b"x"), &lim, &ok, &dir).0.is_ok());
        assert!(run_stream(&make_zip("pulsetray.exe", &fake_pe(0x8664, b"x")), &lim, &ok, &dir).0.is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_download_over_the_limit_stops() {
        let dir = temp("stream_big");
        let data = fake_pe(0x8664, &vec![0u8; 5000]);
        let lim = Limits { sha256: None, size: None, total_hint: 0, max: 1000 };
        assert_eq!(run_stream(&data, &lim, &AtomicBool::new(false), &dir).0, Err(UpdErr::TooLarge));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stop_request_ends_the_download_between_two_chunks() {
        let dir = temp("stream_cancel");
        let data = fake_pe(0x8664, &vec![1u8; 1_000_000]);
        let cancel = AtomicBool::new(false);
        let lim = Limits { sha256: None, size: None, total_hint: data.len() as u64, max: MAX_PKG };
        let chunks = AtomicUsize::new(0);
        let mut r = Pieces { data: &data, step: 50_000, on_read: Some(Box::new(|n| if n == 3 { cancel.store(true, Ordering::SeqCst) })), reads: 0 };
        let got = stream_to_file(&mut r, &dir.join("c.part"), &lim, &cancel, &mut |_, _| {
            chunks.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(got, Err(UpdErr::Cancelled));
        assert!(chunks.load(Ordering::SeqCst) <= 3, "it stopped, it did not run on: {}", chunks.load(Ordering::SeqCst));
        assert!(r.reads <= 4);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn progress_is_thinned_out() {
        let mut t = Throttle::new(Duration::from_millis(200));
        let t0 = Instant::now();
        assert!(t.due(t0), "the first one always goes");
        assert!(!t.due(t0 + Duration::from_millis(50)));
        assert!(!t.due(t0 + Duration::from_millis(199)));
        assert!(t.due(t0 + Duration::from_millis(200)));
        assert!(!t.due(t0 + Duration::from_millis(250)));
        let sent = (0..1000).filter(|i| t.due(t0 + Duration::from_millis(400 + i * 10))).count();
        assert!((45..=51).contains(&sent), "10 s of chunks every 10 ms -> about 5 per second: {sent}");
    }

    // ---- installing

    fn fake_download(dir: &Path, data: &[u8]) -> Downloaded {
        let path = dir.join("PulseTray_Update.pkg");
        std::fs::write(&path, data).unwrap();
        Downloaded { version: "2.0.0".into(), path, sha256: sha_hex(data) }
    }

    #[test]
    fn installs_a_bare_exe_and_a_zip_alike() {
        for zipped in [false, true] {
            let dir = temp(if zipped { "inst_zip" } else { "inst_exe" });
            let exe = dir.join("pulsetray.exe");
            std::fs::write(&exe, fake_pe(0x8664, b"-old")).unwrap();
            let new = fake_pe(0x8664, b"-new");
            let pkg = fake_download(&dir, &if zipped { make_zip("PulseTray.exe", &new) } else { new.clone() });
            install_into(&exe, &pkg).unwrap();
            assert_eq!(std::fs::read(&exe).unwrap(), new);
            assert_eq!(std::fs::read(with_suffix(&exe, ".old")).unwrap(), fake_pe(0x8664, b"-old"), "the previous version is kept for the guard");
            assert!(!pkg.path.exists(), "the package is used up");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_package_that_changed_on_disk_is_not_installed() {
        let dir = temp("inst_tamper");
        let exe = dir.join("pulsetray.exe");
        std::fs::write(&exe, fake_pe(0x8664, b"-old")).unwrap();
        let pkg = fake_download(&dir, &fake_pe(0x8664, b"-new"));
        std::fs::write(&pkg.path, fake_pe(0x8664, b"-evil")).unwrap(); // swapped in %TEMP% after the download
        assert_eq!(install_into(&exe, &pkg), Err(UpdErr::Checksum));
        assert_eq!(std::fs::read(&exe).unwrap(), fake_pe(0x8664, b"-old"), "the working exe is untouched");
        assert!(!with_suffix(&exe, ".old").exists() && !with_suffix(&exe, ".new").exists());
        assert!(!pkg.path.exists(), "the bad file is dropped");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_package_for_another_cpu_or_without_the_exe_leaves_the_app_alone() {
        let dir = temp("inst_arch");
        let exe = dir.join("pulsetray.exe");
        let old = fake_pe(0x8664, b"-old");
        std::fs::write(&exe, &old).unwrap();
        let arm = fake_download(&dir, &fake_pe(0xAA64, b"-arm"));
        assert_eq!(install_into(&exe, &arm), Err(UpdErr::Arch));
        let empty = fake_download(&dir, &make_zip("readme.txt", b"hello"));
        assert!(matches!(install_into(&exe, &empty), Err(UpdErr::Package(_))));
        let junk = fake_download(&dir, b"<html>");
        assert!(matches!(install_into(&exe, &junk), Err(UpdErr::Package(_))));
        let gone = Downloaded { version: "2.0.0".into(), path: dir.join("missing.pkg"), sha256: "0".repeat(64) };
        assert!(matches!(install_into(&exe, &gone), Err(UpdErr::Io(_))));
        assert_eq!(std::fs::read(&exe).unwrap(), old);
        assert!(!with_suffix(&exe, ".old").exists() && !with_suffix(&exe, ".new").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn leftovers_of_an_earlier_run_are_removed() {
        let dir = temp("stale");
        let pkg = dir.join("PulseTray_Update.pkg");
        std::fs::write(&pkg, b"MZ").unwrap();
        std::fs::write(with_suffix(&pkg, ".part"), b"MZ").unwrap();
        remove_package_files(&pkg);
        assert!(!pkg.exists() && !with_suffix(&pkg, ".part").exists());
        remove_package_files(&pkg); // nothing there: no error, no panic
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writability_is_checked_before_anything_is_downloaded() {
        let dir = temp("writable");
        assert!(check_writable(&dir.join("pulsetray.exe")).is_ok());
        assert!(!dir.join(format!("{APP_ID}.write-test")).exists(), "the probe file is gone again");
        assert!(matches!(check_writable(&dir.join("no-such-folder").join("pulsetray.exe")), Err(UpdErr::NotWritable(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_reach_the_ui_as_a_code_and_a_detail() {
        assert_eq!(UpdErr::Checksum.ui(), "checksum");
        assert_eq!(UpdErr::Http(503).ui(), "http: 503");
        assert_eq!(UpdErr::Network("timed out".into()).ui(), "network: timed out");
        assert_eq!(UpdErr::NotWritable("denied".into()).ui(), "not_writable: denied");
        assert_eq!(UpdErr::Cancelled.ui(), "cancelled");
    }

    // ---- command line

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line_after_an_update() {
        let a = relaunch_args(4242, "1.0.0");
        assert_eq!(a, args(&["--minimized", "--wait-pid", "4242", "--updated", "1.0.0"]));
        // what the new process reads back (the exe name comes first on a real command line)
        let mut full = args(&["C:\\Apps\\pulsetray.exe"]);
        full.extend(a);
        assert_eq!(arg_value(&full, "--wait-pid").and_then(|p| p.parse::<u32>().ok()), Some(4242));
        assert_eq!(updated_from(&full), Some("1.0.0".into()));
        assert!(full.iter().any(|x| x == "--minimized"));
    }

    #[test]
    fn updated_from_accepts_only_a_plain_version() {
        assert_eq!(updated_from(&args(&["x"])), None);
        assert_eq!(updated_from(&args(&["x", "--updated"])), None, "no value");
        assert_eq!(updated_from(&args(&["x", "--updated", "v1.2.3"])), Some("1.2.3".into()));
        assert_eq!(updated_from(&args(&["x", "--minimized", "--updated", "10.0"])), Some("10.0".into()));
        for bad in ["", "abc", "1.0.0; calc", "1.0.0-beta", &"1".repeat(40), "<b>1</b>"] {
            assert_eq!(updated_from(&args(&["x", "--updated", bad])), None, "{bad}");
        }
        assert_eq!(arg_value(&args(&["x", "--wait-pid", "12"]), "--wait-pid"), Some("12".into()));
        assert_eq!(arg_value(&args(&["x", "--wait-pid"]), "--wait-pid"), None);
    }
}
