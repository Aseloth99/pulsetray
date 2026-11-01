//! Per-process CPU, memory and disk I/O from one `NtQuerySystemInformation(SystemProcessInformation)` call.
//!
//! This is where Task Manager gets them too. Opening every process (what sysinfo does) fails for protected
//! ones, and those then read as 0 %: `vmmem` (WSL 2 / Hyper-V) can use most of the CPU and still not show up.
//! The system call needs no handles and returns every process at once, which is also much cheaper.

use std::collections::HashMap;
use std::time::Instant;

/// One process, measured over the time since the previous reading.
#[derive(Debug, Clone)]
pub struct ProcNow {
    pub pid: u32,
    pub name: String,
    /// Share of the whole CPU (all cores together), 0..100.
    pub cpu: f32,
    /// Private working set, the "Memory" column of Task Manager (bytes).
    pub mem: u64,
    /// Bytes read + written per second.
    pub disk: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Raw {
    pid: u32,
    /// The process that started it (may be gone already; pids are reused, so only a hint).
    parent: u32,
    name: String,
    /// Start time: with the pid it identifies a process (pids are reused).
    create: i64,
    /// User + kernel time, 100 ns units.
    cpu: u64,
    mem: u64,
    /// Bytes read + written since start.
    io: u64,
}

#[derive(Default)]
pub struct ProcTable {
    prev: HashMap<u32, (i64, u64, u64)>,
    at: Option<Instant>,
    /// Reused between calls; u64 keeps it 8-byte aligned, which the system call needs.
    buf: Vec<u64>,
}

impl ProcTable {
    /// `None` on the first call (and when the system call fails): CPU and I/O are rates and need a previous reading.
    pub fn sample(&mut self, cores: usize) -> Option<Vec<ProcNow>> {
        let raw = query(&mut self.buf)?;
        let now = Instant::now();
        let prev_at = self.at.replace(now);
        let prev = std::mem::take(&mut self.prev);
        self.prev = raw.iter().map(|r| (r.pid, (r.create, r.cpu, r.io))).collect();
        let dt = now.duration_since(prev_at?).as_secs_f64();
        (dt > 0.0).then(|| rates(&raw, &prev, dt, cores))
    }
}

/// Every process right now (pid, parent, start time, name), for checks that must not trust a list that is a
/// few seconds old: pids are reused.
pub fn list_now() -> Option<Vec<(u32, u32, i64, String)>> {
    let mut buf = Vec::new();
    Some(query(&mut buf)?.into_iter().map(|r| (r.pid, r.parent, r.create, r.name)).collect())
}

/// `root` and everything it started, directly or further down (a reused parent pid doesn't count: the child
/// must have started after its parent).
pub fn descendants(list: &[(u32, u32, i64, String)], root: u32) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::from([root]);
    let started: HashMap<u32, i64> = list.iter().map(|p| (p.0, p.2)).collect();
    loop {
        let before = out.len();
        for &(pid, parent, create, _) in list {
            if !out.contains(&pid) && out.contains(&parent) && started.get(&parent).is_none_or(|&pc| pc <= create) {
                out.insert(pid);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// Page file in use and its current size, all page files together (bytes): what really sits in the page file,
/// not "commit charge minus RAM" (which is what sysinfo reports as swap and can be ten gigabytes too high).
#[cfg(windows)]
pub fn pagefile() -> Option<(u64, u64)> {
    use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SYSTEM_INFORMATION_CLASS};
    use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    const SYSTEM_PAGEFILE_INFORMATION: SYSTEM_INFORMATION_CLASS = SYSTEM_INFORMATION_CLASS(18);
    let mut buf = vec![0u64; 512]; // 4 KB: room for every page file and its name
    let mut needed = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(SYSTEM_PAGEFILE_INFORMATION, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut needed)
    };
    if status.0 < 0 {
        return None;
    }
    let mut info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut info) };
    let used = (needed as usize).min(buf.len() * 8);
    let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), used) };
    Some(parse_pagefiles(bytes, info.dwPageSize.max(4096) as u64))
}

#[cfg(not(windows))]
pub fn pagefile() -> Option<(u64, u64)> {
    None
}

/// SYSTEM_PAGEFILE_INFORMATION entries: NextEntryOffset, TotalSize, TotalInUse, PeakUsage (pages), then the name.
/// Returns (total, in use) in bytes; no page file at all = (0, 0).
fn parse_pagefiles(b: &[u8], page: u64) -> (u64, u64) {
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as u64;
    let (mut total, mut in_use, mut off) = (0u64, 0u64, 0usize);
    while off + 16 <= b.len() {
        total += u32_at(off + 4) * page;
        in_use += u32_at(off + 8) * page;
        let next = u32_at(off) as usize;
        if next == 0 {
            break;
        }
        off += next;
    }
    (total, in_use.min(total))
}

fn rates(raw: &[Raw], prev: &HashMap<u32, (i64, u64, u64)>, dt: f64, cores: usize) -> Vec<ProcNow> {
    // CPU time all cores together could have spent in `dt`, in 100 ns units.
    let budget = dt * 1e7 * cores.max(1) as f64;
    raw.iter()
        .map(|r| {
            // Same pid and start time = the same process; otherwise it started since the last reading.
            let (cpu0, io0) = match prev.get(&r.pid) {
                Some(&(create, cpu, io)) if create == r.create => (cpu, io),
                _ => (0, 0),
            };
            ProcNow {
                pid: r.pid,
                name: r.name.clone(),
                cpu: (r.cpu.saturating_sub(cpu0) as f64 / budget * 100.0).min(100.0) as f32,
                mem: r.mem,
                disk: r.io.saturating_sub(io0) as f64 / dt,
            }
        })
        .collect()
}

#[cfg(windows)]
fn query(buf: &mut Vec<u64>) -> Option<Vec<Raw>> {
    use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
    if buf.len() < 64 * 1024 {
        buf.resize(64 * 1024, 0); // 512 KB: enough for a few hundred processes and their threads
    }
    for _ in 0..6 {
        let bytes = (buf.len() * 8) as u32;
        let mut needed = 0u32;
        let status = unsafe { NtQuerySystemInformation(SystemProcessInformation, buf.as_mut_ptr().cast(), bytes, &mut needed) };
        if status.0 == STATUS_INFO_LENGTH_MISMATCH {
            // Processes come and go between the calls: leave some headroom.
            buf.resize((needed as usize).max(bytes as usize * 2) / 8 + 8 * 1024, 0);
            continue;
        }
        if status.0 < 0 {
            return None;
        }
        let used = (needed as usize).min(buf.len() * 8);
        let base = buf.as_ptr() as usize;
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), used) };
        return Some(parse(bytes, base));
    }
    None
}

#[cfg(not(windows))]
fn query(_buf: &mut Vec<u64>) -> Option<Vec<Raw>> {
    None
}

// SYSTEM_PROCESS_INFORMATION, 64-bit layout (stable since Windows 7; the documented struct hides the times
// in "Reserved1"). Each entry is followed by its thread records; NextEntryOffset jumps over them.
const NEXT: usize = 0x00;
const PRIVATE_WS: usize = 0x08;
const CREATE_TIME: usize = 0x20;
const USER_TIME: usize = 0x28;
const KERNEL_TIME: usize = 0x30;
const NAME_LEN: usize = 0x38;
const NAME_PTR: usize = 0x40;
const PID: usize = 0x50;
const PARENT: usize = 0x58;
const WORKING_SET: usize = 0x90;
const READ_BYTES: usize = 0xE8;
const WRITE_BYTES: usize = 0xF0;
const ENTRY: usize = 0x100;

/// `base` = the buffer's address: image names point into the same buffer.
fn parse(b: &[u8], base: usize) -> Vec<Raw> {
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + ENTRY <= b.len() {
        let pid = u64_at(off + PID) as u32;
        // pid 0 is the "System Idle Process": idle time, not a program.
        if pid != 0 {
            let len = u16_at(off + NAME_LEN) as usize;
            let ptr = u64_at(off + NAME_PTR) as usize;
            let name = match ptr.checked_sub(base) {
                Some(start) if len > 0 && start + len <= b.len() => {
                    let units: Vec<u16> = (0..len / 2).map(|i| u16_at(start + i * 2)).collect();
                    String::from_utf16_lossy(&units)
                }
                _ => if pid == 4 { "System".into() } else { format!("pid {pid}") },
            };
            let private = u64_at(off + PRIVATE_WS);
            out.push(Raw {
                pid,
                parent: u64_at(off + PARENT) as u32,
                name,
                create: u64_at(off + CREATE_TIME) as i64,
                cpu: u64_at(off + USER_TIME).saturating_add(u64_at(off + KERNEL_TIME)),
                mem: if private > 0 { private } else { u64_at(off + WORKING_SET) },
                io: u64_at(off + READ_BYTES).saturating_add(u64_at(off + WRITE_BYTES)),
            });
        }
        let next = u32_at(off + NEXT) as usize;
        if next == 0 {
            break;
        }
        off += next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two entries laid out like the system call does it: fixed part, room for threads, the names at the end.
    fn fake_buffer() -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        let base = b.as_ptr() as usize;
        let put = |b: &mut Vec<u8>, o: usize, v: &[u8]| b[o..o + v.len()].copy_from_slice(v);
        let name = |b: &mut Vec<u8>, entry: usize, at: usize, s: &str| {
            let units: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
            put(b, entry + NAME_LEN, &(units.len() as u16).to_le_bytes());
            put(b, entry + NAME_PTR, &((base + at) as u64).to_le_bytes());
            put(b, at, &units);
        };
        // entry 1 at 0x000: vmmem, pid 6300; next entry 0x180 further (as if it had threads)
        put(&mut b, NEXT, &0x180u32.to_le_bytes());
        put(&mut b, PID, &6300u64.to_le_bytes());
        put(&mut b, PARENT, &812u64.to_le_bytes());
        put(&mut b, CREATE_TIME, &111i64.to_le_bytes());
        put(&mut b, USER_TIME, &4_000_000u64.to_le_bytes());
        put(&mut b, KERNEL_TIME, &1_000_000u64.to_le_bytes());
        put(&mut b, PRIVATE_WS, &(8u64 << 30).to_le_bytes());
        put(&mut b, READ_BYTES, &100u64.to_le_bytes());
        put(&mut b, WRITE_BYTES, &50u64.to_le_bytes());
        name(&mut b, 0, 0x300, "vmmem");
        // entry 2 at 0x180: no private working set reported, last entry
        let e = 0x180;
        put(&mut b, e + PID, &42u64.to_le_bytes());
        put(&mut b, e + WORKING_SET, &4096u64.to_le_bytes());
        name(&mut b, e, 0x340, "çay.exe");
        b
    }

    #[test]
    fn parse_reads_names_times_memory_and_io() {
        let b = fake_buffer();
        let raw = parse(&b, b.as_ptr() as usize);
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0], Raw { pid: 6300, parent: 812, name: "vmmem".into(), create: 111, cpu: 5_000_000, mem: 8 << 30, io: 150 });
        assert_eq!(raw[1].name, "çay.exe");
        assert_eq!(raw[1].mem, 4096, "falls back to the working set");
    }

    #[test]
    fn a_name_pointing_outside_the_buffer_is_not_read() {
        let b = fake_buffer();
        // Pretend the buffer lives elsewhere: every name pointer is now out of range.
        let raw = parse(&b, b.as_ptr() as usize + 0x10_000);
        assert_eq!(raw[0].name, "pid 6300");
    }

    #[test]
    fn cpu_share_is_of_all_cores_and_a_reused_pid_starts_from_zero() {
        let now = vec![
            Raw { pid: 1, parent: 0, name: "a".into(), create: 10, cpu: 30_000_000, mem: 0, io: 2_000 },
            Raw { pid: 2, parent: 0, name: "b".into(), create: 99, cpu: 1_000_000, mem: 0, io: 0 },
        ];
        // pid 1 used 2 s of CPU time in 1 s on a 4-core machine = 50 % of the whole CPU; pid 2 is a new process
        // that reuses an old pid (other start time): all of its 0.1 s counts.
        let prev = HashMap::from([(1, (10, 10_000_000, 1_000)), (2, (5, 900_000_000, 0))]);
        let r = rates(&now, &prev, 1.0, 4);
        assert!((r[0].cpu - 50.0).abs() < 1e-3, "{}", r[0].cpu);
        assert!((r[0].disk - 1000.0).abs() < 1e-6);
        assert!((r[1].cpu - 2.5).abs() < 1e-3, "{}", r[1].cpu);
    }

    #[test]
    fn page_files_add_up_in_bytes() {
        // Two page files: 1000 pages with 10 in use, then 500 pages with 600 "in use" (clamped to the total).
        let mut b = vec![0u8; 64];
        b[0..4].copy_from_slice(&32u32.to_le_bytes());
        b[4..8].copy_from_slice(&1000u32.to_le_bytes());
        b[8..12].copy_from_slice(&10u32.to_le_bytes());
        b[36..40].copy_from_slice(&500u32.to_le_bytes());
        b[40..44].copy_from_slice(&600u32.to_le_bytes());
        assert_eq!(parse_pagefiles(&b, 4096), (1500 * 4096, 610 * 4096));
        assert_eq!(parse_pagefiles(&[], 4096), (0, 0), "no page file");
    }

    #[test]
    #[cfg(windows)]
    fn the_live_page_file_is_not_bigger_than_it_is() {
        let (total, used) = pagefile().expect("the system call answers");
        assert!(used <= total);
        println!("page file {} MB, in use {} MB", total >> 20, used >> 20);
    }

    #[test]
    fn descendants_follow_children_but_not_a_reused_parent_pid() {
        let list = vec![
            (10, 1, 100, "pulsetray.exe".to_string()),
            (11, 10, 110, "msedgewebview2.exe".to_string()),
            (12, 11, 120, "msedgewebview2.exe".to_string()),
            // pid 10 was reused: this one's "parent" died long before pid 10 started.
            (13, 10, 50, "old.exe".to_string()),
            (14, 1, 130, "other.exe".to_string()),
        ];
        let d = descendants(&list, 10);
        assert_eq!(d, std::collections::HashSet::from([10, 11, 12]));
    }

    #[test]
    #[ignore = "prints this machine's busiest processes: run with --ignored --nocapture"]
    fn print_top_processes() {
        let mut t = ProcTable::default();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        t.sample(cores);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let start = Instant::now();
        let mut list = t.sample(cores).unwrap();
        let cost = start.elapsed();
        list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        for p in list.iter().take(6) {
            println!("{:<28} pid {:>6}  cpu {:>5.1}%  mem {:>6} MB", p.name, p.pid, p.cpu, p.mem >> 20);
        }
        println!("{} processes, one reading took {:.2} ms", list.len(), cost.as_secs_f64() * 1e3);
    }

    #[test]
    #[cfg(windows)]
    fn the_live_table_sees_this_process_working() {
        let mut t = ProcTable::default();
        assert!(t.sample(1).is_none(), "first reading has no baseline");
        let start = Instant::now();
        let mut x = 0u64;
        while start.elapsed().as_millis() < 300 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        std::hint::black_box(x);
        let list = t.sample(1).expect("second reading");
        let me = list.iter().find(|p| p.pid == std::process::id()).expect("own process listed");
        assert!(me.cpu > 10.0, "busy loop should show up, got {}", me.cpu);
        assert!(me.mem > 0);
        assert!(list.iter().any(|p| p.name.eq_ignore_ascii_case("System")), "System is listed");
    }
}
