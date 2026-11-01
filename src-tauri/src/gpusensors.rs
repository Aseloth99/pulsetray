#![allow(unsafe_op_in_unsafe_fn)]
//! GPU clocks / temperature / fan / power.
//!
//! * Windows' own WDDM perf interface (D3DKMT, what Task Manager uses): temperature, fan, memory and
//!   core clock. Works for any driver that implements it; integrated GPUs often leave fields at 0.
//! * NVIDIA's NVML (nvml.dll ships with the driver): adds power draw / power limit and fills gaps.
//!
//! Every value is optional; a field that a driver doesn't report simply stays None and the UI hides it.

use serde::Serialize;
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, D3DKMT_ADAPTERADDRESS,
    D3DKMT_ADAPTER_PERFDATA,
    D3DKMT_CLOSEADAPTER, D3DKMT_NODE_PERFDATA, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
    KMTQAITYPE_ADAPTERADDRESS, KMTQAITYPE_ADAPTERPERFDATA, KMTQAITYPE_NODEPERFDATA,
};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, LUID};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GpuSensors {
    pub temp_c: Option<f32>,
    pub core_mhz: Option<u32>,
    pub mem_mhz: Option<u32>,
    pub fan_rpm: Option<u32>,
    pub fan_pct: Option<u32>,
    pub power_w: Option<f32>,
    pub power_limit_w: Option<f32>,
}

/// `luid_0x00000000_0x000145a3` -> LUID
fn parse_luid(s: &str) -> Option<LUID> {
    let mut it = s.strip_prefix("luid_")?.split('_');
    let high = u32::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    let low = u32::from_str_radix(it.next()?.trim_start_matches("0x"), 16).ok()?;
    Some(LUID { LowPart: low, HighPart: high as i32 })
}

unsafe fn query<T>(adapter: u32, kind: windows::Wdk::Graphics::Direct3D::KMTQUERYADAPTERINFOTYPE, data: &mut T) -> bool {
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: adapter,
        Type: kind,
        pPrivateDriverData: data as *mut T as *mut c_void,
        PrivateDriverDataSize: std::mem::size_of::<T>() as u32,
    };
    D3DKMTQueryAdapterInfo(&mut q).0 >= 0
}

/// PCI location (bus, device, function) of an adapter. Windows can list one physical card under several
/// LUIDs (virtual display drivers, game-streaming tools); they all share this address.
pub fn pci_address(luid: &str) -> Option<(u32, u32, u32)> {
    let l = parse_luid(luid)?;
    unsafe {
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: l, hAdapter: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open).0 < 0 {
            return None;
        }
        let mut a: D3DKMT_ADAPTERADDRESS = std::mem::zeroed();
        let ok = query(open.hAdapter, KMTQAITYPE_ADAPTERADDRESS, &mut a);
        let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: open.hAdapter });
        ok.then_some((a.BusNumber, a.DeviceNumber, a.FunctionNumber))
    }
}

fn from_wddm(luid: &str) -> GpuSensors {
    let mut out = GpuSensors::default();
    let Some(l) = parse_luid(luid) else { return out };
    unsafe {
        let mut open = D3DKMT_OPENADAPTERFROMLUID { AdapterLuid: l, hAdapter: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open).0 < 0 {
            return out;
        }
        let mut perf: D3DKMT_ADAPTER_PERFDATA = std::mem::zeroed();
        if query(open.hAdapter, KMTQAITYPE_ADAPTERPERFDATA, &mut perf) {
            // Temperature is in tenths of a degree Celsius, frequencies in Hz.
            if perf.Temperature > 0 && perf.Temperature < 2000 {
                out.temp_c = Some(perf.Temperature as f32 / 10.0);
            }
            if perf.MemoryFrequency > 0 {
                out.mem_mhz = Some((perf.MemoryFrequency / 1_000_000) as u32);
            }
            if perf.FanRPM > 0 {
                out.fan_rpm = Some(perf.FanRPM);
            }
        }
        // Node 0 is the 3D / graphics engine on every driver I know of.
        let mut node: D3DKMT_NODE_PERFDATA = std::mem::zeroed();
        node.NodeOrdinal = 0;
        if query(open.hAdapter, KMTQAITYPE_NODEPERFDATA, &mut node) && node.Frequency > 0 {
            out.core_mhz = Some((node.Frequency / 1_000_000) as u32);
        }
        let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: open.hAdapter });
    }
    out
}

// ---------------------------------------------------------------- NVML (NVIDIA)

type Dev = *mut c_void;
type FnInit = unsafe extern "C" fn() -> i32;
type FnCount = unsafe extern "C" fn(*mut u32) -> i32;
type FnHandle = unsafe extern "C" fn(u32, *mut Dev) -> i32;
type FnName = unsafe extern "C" fn(Dev, *mut c_char, u32) -> i32;
type FnTemp = unsafe extern "C" fn(Dev, i32, *mut u32) -> i32;
type FnU32 = unsafe extern "C" fn(Dev, *mut u32) -> i32;
type FnClock = unsafe extern "C" fn(Dev, i32, *mut u32) -> i32;

struct Nvml {
    _lib: HMODULE,
    count: FnCount,
    handle: FnHandle,
    name: FnName,
    temp: FnTemp,
    power: FnU32,
    limit: FnU32,
    fan: FnU32,
    clock: FnClock,
}

// The library handle and function pointers are process-wide and thread-safe to call (NVML is).
unsafe impl Send for Nvml {}
unsafe impl Sync for Nvml {}

unsafe fn sym<T: Copy>(lib: HMODULE, name: &[u8]) -> Option<T> {
    let p = GetProcAddress(lib, windows::core::PCSTR(name.as_ptr()))?;
    Some(std::mem::transmute_copy::<_, T>(&p))
}

fn load_nvml() -> Option<Nvml> {
    unsafe {
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let lib = ["nvml.dll", "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll"]
            .iter()
            .find_map(|p| LoadLibraryW(PCWSTR(wide(p).as_ptr())).ok())?;
        let init: FnInit = sym(lib, b"nvmlInit_v2\0")?;
        if init() != 0 {
            let _ = FreeLibrary(lib);
            return None;
        }
        Some(Nvml {
            _lib: lib,
            count: sym(lib, b"nvmlDeviceGetCount_v2\0")?,
            handle: sym(lib, b"nvmlDeviceGetHandleByIndex_v2\0")?,
            name: sym(lib, b"nvmlDeviceGetName\0")?,
            temp: sym(lib, b"nvmlDeviceGetTemperature\0")?,
            power: sym(lib, b"nvmlDeviceGetPowerUsage\0")?,
            limit: sym(lib, b"nvmlDeviceGetPowerManagementLimit\0")?,
            fan: sym(lib, b"nvmlDeviceGetFanSpeed\0")?,
            clock: sym(lib, b"nvmlDeviceGetClockInfo\0")?,
        })
    }
}

static NVML: OnceLock<Option<Nvml>> = OnceLock::new();

fn norm(s: &str) -> String {
    s.to_lowercase().replace("nvidia", "").replace("geforce", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Which NVML device belongs to this DXGI adapter? NVML and DXGI enumerate in different orders, so
/// without a shared id the only safe link is the name, and only when it is unambiguous.
/// `dxgi_nvidia` = every NVIDIA adapter name DXGI reported, `nvml` = every device name NVML reported.
pub fn pick_device(gpu_name: &str, dxgi_nvidia: &[String], nvml: &[String]) -> Option<usize> {
    // One NVIDIA card on each side: nothing to confuse.
    if dxgi_nvidia.len() == 1 && nvml.len() == 1 {
        return Some(0);
    }
    let me = norm(gpu_name);
    if me.is_empty() {
        return None;
    }
    // Two identical cards cannot be told apart by name: better no NVML data than the wrong card's.
    if dxgi_nvidia.iter().filter(|n| norm(n) == me).count() != 1 {
        return None;
    }
    let mut hits = nvml.iter().enumerate().filter(|(_, n)| norm(n) == me).map(|(i, _)| i);
    match (hits.next(), hits.next()) {
        (Some(i), None) => Some(i),
        _ => None,
    }
}

fn from_nvml(gpu_name: &str, dxgi_nvidia: &[String]) -> Option<GpuSensors> {
    let n = NVML.get_or_init(load_nvml).as_ref()?;
    unsafe {
        let mut count = 0u32;
        if (n.count)(&mut count) != 0 || count == 0 {
            return None;
        }
        let mut devs: Vec<Dev> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for i in 0..count.min(16) {
            let mut dev: Dev = std::ptr::null_mut();
            if (n.handle)(i, &mut dev) != 0 {
                continue;
            }
            let mut buf = [0 as c_char; 96];
            let name = if (n.name)(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().to_string()
            } else {
                String::new()
            };
            devs.push(dev);
            names.push(name);
        }
        let dev = devs[pick_device(gpu_name, dxgi_nvidia, &names)?];

        let mut s = GpuSensors::default();
        let mut v = 0u32;
        if (n.temp)(dev, 0, &mut v) == 0 {
            s.temp_c = Some(v as f32);
        }
        if (n.power)(dev, &mut v) == 0 {
            s.power_w = Some(v as f32 / 1000.0);
        }
        if (n.limit)(dev, &mut v) == 0 {
            s.power_limit_w = Some(v as f32 / 1000.0);
        }
        if (n.fan)(dev, &mut v) == 0 {
            s.fan_pct = Some(v);
        }
        if (n.clock)(dev, 0, &mut v) == 0 {
            s.core_mhz = Some(v);
        }
        if (n.clock)(dev, 2, &mut v) == 0 {
            s.mem_mhz = Some(v);
        }
        Some(s)
    }
}

/// Merge what WDDM and (for NVIDIA) NVML know about one adapter.
pub fn read(luid: &str, name: &str, dxgi_nvidia: &[String]) -> GpuSensors {
    let mut s = from_wddm(luid);
    if name.to_lowercase().contains("nvidia") {
        if let Some(n) = from_nvml(name, dxgi_nvidia) {
            // NVML is authoritative for NVIDIA; keep WDDM values only where NVML had none.
            s = GpuSensors {
                temp_c: n.temp_c.or(s.temp_c),
                core_mhz: n.core_mhz.or(s.core_mhz),
                mem_mhz: n.mem_mhz.or(s.mem_mhz),
                fan_rpm: s.fan_rpm,
                fan_pct: n.fan_pct,
                power_w: n.power_w,
                power_limit_w: n.power_limit_w,
            };
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn single_card_matches_even_if_the_names_differ() {
        assert_eq!(pick_device("NVIDIA GeForce RTX 3060 Laptop GPU", &v(&["NVIDIA GeForce RTX 3060 Laptop GPU"]), &v(&["NVIDIA GeForce RTX 3060 Laptop GPU"])), Some(0));
        assert_eq!(pick_device("NVIDIA GeForce RTX 3060", &v(&["NVIDIA GeForce RTX 3060"]), &v(&["something else"])), Some(0));
    }

    #[test]
    fn two_different_cards_match_by_name() {
        let dxgi = v(&["NVIDIA GeForce RTX 4090", "NVIDIA GeForce RTX 3060"]);
        let nvml = v(&["NVIDIA GeForce RTX 3060", "NVIDIA GeForce RTX 4090"]); // other order than DXGI
        assert_eq!(pick_device("NVIDIA GeForce RTX 4090", &dxgi, &nvml), Some(1));
        assert_eq!(pick_device("NVIDIA GeForce RTX 3060", &dxgi, &nvml), Some(0));
    }

    #[test]
    fn two_identical_cards_get_no_nvml_data_rather_than_the_wrong_cards() {
        let two = v(&["NVIDIA GeForce RTX 3060", "NVIDIA GeForce RTX 3060"]);
        assert_eq!(pick_device("NVIDIA GeForce RTX 3060", &two, &two), None);
    }

    #[test]
    fn empty_or_unknown_names_never_match() {
        let dxgi = v(&["NVIDIA GeForce RTX 4090", "NVIDIA GeForce RTX 3060"]);
        assert_eq!(pick_device("NVIDIA GeForce RTX 4090", &dxgi, &v(&["", ""])), None);
        assert_eq!(pick_device("", &dxgi, &v(&["NVIDIA GeForce RTX 4090", "NVIDIA GeForce RTX 3060"])), None);
    }

    #[test]
    #[ignore = "diagnostic: run with --ignored --nocapture"]
    fn live_gpu_sensors() {
        let all: Vec<String> = crate::hardware::dxgi_gpus().into_iter().map(|g| g.name).collect();
        for g in crate::hardware::dxgi_gpus() {
            println!("{} ({})
   {:?}", g.name, g.luid, read(&g.luid, &g.name, &all));
        }
    }
}
