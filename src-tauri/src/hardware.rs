//! Static hardware facts (CPU model, RAM modules, GPUs, drives, adapters). Read once at startup;
//! WMI is slow-ish (hundreds of ms) so it runs on its own thread.

use serde::{Deserialize, Serialize};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use wmi::WMIConnection;

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuStatic {
    /// Matches the PDH instance prefix, e.g. `luid_0x00000000_0x000145a3`.
    pub luid: String,
    pub name: String,
    pub vram_total: u64,
    pub shared_total: u64,
    /// Other LUIDs Windows lists for the same physical card (their counters are merged in).
    #[serde(skip)]
    pub alias_luids: Vec<String>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemModule {
    pub capacity: u64,
    pub rated_mts: u32,
    pub configured_mts: u32,
    pub kind: String,
    pub maker: String,
    pub slot: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DriveStatic {
    pub model: String,
    pub size: u64,
    /// "SSD", "HDD" or "".
    pub media: String,
    /// "NVMe", "SATA", "USB" or "".
    pub bus: String,
    /// \\.\PhysicalDriveN, to match the health readings (diskhealth.rs).
    pub index: Option<u32>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdapterStatic {
    pub name: String,
    pub description: String,
    pub link_mbps: u64,
    /// "wifi", "ethernet" or "other".
    pub kind: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    pub cpu_name: String,
    pub cpu_max_mhz: u32,
    pub cpu_cores: u32,
    pub cpu_logical: u32,
    pub cpu_l2_kb: u32,
    pub cpu_l3_kb: u32,
    pub mem_modules: Vec<MemModule>,
    pub mem_slots: u32,
    pub board: String,
    pub os: String,
    pub os_build: String,
    pub gpus: Vec<GpuStatic>,
    pub drives: Vec<DriveStatic>,
    pub adapters: Vec<AdapterStatic>,
}

/// GPUs through DXGI: accurate VRAM size (WMI caps AdapterRAM at 4 GB) and the LUID used by the counters.
pub fn dxgi_gpus() -> Vec<GpuStatic> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = adapter.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let end = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
            out.push(GpuStatic {
                luid: format!(
                    "luid_0x{:08x}_0x{:08x}",
                    d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart
                ),
                name: String::from_utf16_lossy(&d.Description[..end]).trim().to_string(),
                vram_total: d.DedicatedVideoMemory as u64,
                shared_total: d.SharedSystemMemory as u64,
                alias_luids: Vec::new(),
            });
        }
    }
    merge_same_card(out, crate::gpusensors::pci_address)
}

/// A real PCI device number is 0-31 and a function 0-7; virtual display drivers report 0xFFFF (and sometimes
/// bus 0xFFFFFFFF) because there is no hardware behind them.
fn is_virtual_address((bus, device, function): (u32, u32, u32)) -> bool {
    bus == u32::MAX || device > 31 || function > 7
}

/// One physical card can show up as several adapters with different LUIDs. Fold those into one entry
/// (same name and PCI address); the entry with the most dedicated memory is the one that is shown.
fn merge_same_card(
    gpus: Vec<GpuStatic>,
    addr: impl Fn(&str) -> Option<(u32, u32, u32)>,
) -> Vec<GpuStatic> {
    let mut out: Vec<(Option<(u32, u32, u32)>, GpuStatic)> = Vec::new();
    // Virtual display drivers (Sunshine/Moonlight, Parsec...) copy the real card's name but have no PCI
    // location (bus 0xFFFFFFFF, or device/function 0xFFFF). Handle them last, folding each into the real card of the same name.
    let (virt, real): (Vec<_>, Vec<_>) = gpus.into_iter().partition(|g| addr(&g.luid).is_some_and(is_virtual_address));
    for g in real {
        let a = addr(&g.luid);
        match out.iter_mut().find(|(oa, o)| a.is_some() && *oa == a && o.name == g.name) {
            Some((_, o)) => {
                if g.vram_total > o.vram_total {
                    let old = std::mem::replace(o, g);
                    o.alias_luids.push(old.luid);
                    o.alias_luids.extend(old.alias_luids);
                } else {
                    o.alias_luids.push(g.luid);
                }
            }
            None => out.push((a, g)),
        }
    }
    for g in virt {
        match out.iter_mut().find(|(_, o)| o.name == g.name) {
            Some((_, o)) => o.alias_luids.push(g.luid),
            None => out.push((None, g)),
        }
    }
    out.into_iter().map(|(_, g)| g).collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Cpu {
    name: Option<String>,
    max_clock_speed: Option<u32>,
    number_of_cores: Option<u32>,
    number_of_logical_processors: Option<u32>,
    l2_cache_size: Option<u32>,
    l3_cache_size: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PhysMem {
    manufacturer: Option<String>,
    capacity: Option<u64>,
    speed: Option<u32>,
    configured_clock_speed: Option<u32>,
    #[serde(rename = "SMBIOSMemoryType")]
    memory_type: Option<u32>,
    device_locator: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MemArray {
    memory_devices: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Board {
    manufacturer: Option<String>,
    product: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Os {
    caption: Option<String>,
    build_number: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PhysDisk {
    device_id: Option<String>,
    friendly_name: Option<String>,
    media_type: Option<u16>,
    bus_type: Option<u16>,
    size: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NetAdapter {
    name: Option<String>,
    interface_description: Option<String>,
    receive_link_speed: Option<u64>,
    ndis_physical_medium: Option<u32>,
    hardware_interface: Option<bool>,
}

fn mem_kind(code: u32) -> &'static str {
    match code {
        20 => "DDR",
        21 => "DDR2",
        24 => "DDR3",
        26 => "DDR4",
        27 => "LPDDR",
        28 => "LPDDR2",
        29 => "LPDDR3",
        30 => "LPDDR4",
        34 => "DDR5",
        35 => "LPDDR5",
        _ => "",
    }
}

/// Log (stderr) why a WMI query produced nothing; the UI just hides those rows.
fn ok<T>(what: &str, r: wmi::WMIResult<T>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[hardware] {what}: {e}");
            None
        }
    }
}

fn clean(s: Option<String>) -> String {
    s.map(|x| x.trim().to_string()).unwrap_or_default()
}

/// "Micro-Star International Co., Ltd." + "B650 GAMING PLUS WIFI (MS-7E26)" -> "MSI B650 GAMING PLUS WIFI (MS-7E26)".
/// The long legal names filled the System tile's row and cut the model off.
fn board_name(manufacturer: &str, product: &str) -> String {
    let m = manufacturer.trim();
    let low = m.to_lowercase();
    let short = [
        ("micro-star", "MSI"),
        ("asustek", "ASUS"),
        ("gigabyte", "Gigabyte"),
        ("asrock", "ASRock"),
        ("hewlett", "HP"),
        ("dell", "Dell"),
        ("lenovo", "Lenovo"),
        ("biostar", "Biostar"),
    ]
    .iter()
    .find(|(key, _)| low.contains(key))
    .map(|(_, name)| name.to_string())
    .unwrap_or_else(|| {
        let mut s = m.to_string();
        for suffix in [", Ltd.", " Co.", " Ltd.", " Inc.", " INC.", " Corporation", " Corp.", " GmbH", " Technology"] {
            s = s.trim_end_matches(',').trim().trim_end_matches(suffix).trim().to_string();
        }
        s
    });
    let product = product.trim();
    if short.is_empty() || product.to_lowercase().starts_with(&short.to_lowercase()) {
        product.to_string()
    } else {
        format!("{short} {product}").trim().to_string()
    }
}

pub fn load() -> Hardware {
    let mut hw = Hardware { gpus: dxgi_gpus(), ..Default::default() };

    if let Ok(wmi) = WMIConnection::new() {
        if let Some(v) = ok("Cpu", wmi.raw_query::<Cpu>("SELECT Name, MaxClockSpeed, NumberOfCores, NumberOfLogicalProcessors, L2CacheSize, L3CacheSize FROM Win32_Processor")) {
            if let Some(c) = v.into_iter().next() {
                hw.cpu_name = clean(c.name);
                hw.cpu_max_mhz = c.max_clock_speed.unwrap_or(0);
                hw.cpu_cores = c.number_of_cores.unwrap_or(0);
                hw.cpu_logical = c.number_of_logical_processors.unwrap_or(0);
                hw.cpu_l2_kb = c.l2_cache_size.unwrap_or(0);
                hw.cpu_l3_kb = c.l3_cache_size.unwrap_or(0);
            }
        }
        if let Some(v) = ok("PhysMem", wmi.raw_query::<PhysMem>("SELECT Manufacturer, Capacity, Speed, ConfiguredClockSpeed, SMBIOSMemoryType, DeviceLocator FROM Win32_PhysicalMemory")) {
            hw.mem_modules = v
                .into_iter()
                .map(|m| MemModule {
                    capacity: m.capacity.unwrap_or(0),
                    rated_mts: m.speed.unwrap_or(0),
                    configured_mts: m.configured_clock_speed.unwrap_or(0),
                    kind: mem_kind(m.memory_type.unwrap_or(0)).to_string(),
                    maker: clean(m.manufacturer),
                    slot: clean(m.device_locator),
                })
                .collect();
        }
        if let Some(v) = ok("MemArray", wmi.raw_query::<MemArray>("SELECT MemoryDevices FROM Win32_PhysicalMemoryArray")) {
            hw.mem_slots = v.iter().filter_map(|a| a.memory_devices).sum();
        }
        if let Some(v) = ok("Board", wmi.raw_query::<Board>("SELECT Manufacturer, Product FROM Win32_BaseBoard")) {
            if let Some(b) = v.into_iter().next() {
                hw.board = board_name(&clean(b.manufacturer), &clean(b.product));
            }
        }
        if let Some(v) = ok("Os", wmi.raw_query::<Os>("SELECT Caption, BuildNumber FROM Win32_OperatingSystem")) {
            if let Some(o) = v.into_iter().next() {
                hw.os = clean(o.caption).trim_start_matches("Microsoft ").to_string();
                hw.os_build = clean(o.build_number);
            }
        }
    }

    if let Ok(wmi) = WMIConnection::with_namespace_path("ROOT\\Microsoft\\Windows\\Storage") {
        if let Some(v) = ok("PhysDisk", wmi.raw_query::<PhysDisk>("SELECT DeviceId, FriendlyName, MediaType, BusType, Size FROM MSFT_PhysicalDisk")) {
            hw.drives = v
                .into_iter()
                .map(|d| DriveStatic {
                    model: clean(d.friendly_name),
                    size: d.size.unwrap_or(0),
                    media: match d.media_type {
                        Some(3) => "HDD",
                        Some(4) => "SSD",
                        _ => "",
                    }
                    .to_string(),
                    bus: match d.bus_type {
                        Some(17) => "NVMe",
                        Some(11) => "SATA",
                        Some(7) => "USB",
                        Some(8) => "RAID",
                        _ => "",
                    }
                    .to_string(),
                    index: d.device_id.as_deref().and_then(|s| s.trim().parse().ok()),
                })
                .collect();
        }
    }

    if let Ok(wmi) = WMIConnection::with_namespace_path("ROOT\\StandardCimv2") {
        if let Some(v) = ok("NetAdapter", wmi.raw_query::<NetAdapter>("SELECT Name, InterfaceDescription, ReceiveLinkSpeed, NdisPhysicalMedium, HardwareInterface FROM MSFT_NetAdapter")) {
            hw.adapters = v
                .into_iter()
                .filter(|a| a.hardware_interface.unwrap_or(true))
                .map(|a| AdapterStatic {
                    name: clean(a.name),
                    description: clean(a.interface_description),
                    link_mbps: a.receive_link_speed.filter(|bps| *bps < 1_000_000_000_000).unwrap_or(0) / 1_000_000,
                    kind: match a.ndis_physical_medium {
                        Some(9) => "wifi",
                        Some(14) => "ethernet",
                        _ => "other",
                    }
                    .to_string(),
                })
                .collect();
        }
    }
    hw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump() {
        let hw = load();
        println!("{}", serde_json::to_string_pretty(&hw).unwrap());
    }

    #[test]
    fn board_vendor_names_are_shortened() {
        assert_eq!(
            board_name("Micro-Star International Co., Ltd.", "B650 GAMING PLUS WIFI (MS-7E26)"),
            "MSI B650 GAMING PLUS WIFI (MS-7E26)"
        );
        assert_eq!(board_name("ASUSTeK COMPUTER INC.", "ROG STRIX B650E-F GAMING WIFI"), "ASUS ROG STRIX B650E-F GAMING WIFI");
        assert_eq!(board_name("Gigabyte Technology Co., Ltd.", "B550 AORUS ELITE"), "Gigabyte B550 AORUS ELITE");
        assert_eq!(board_name("ASRock", "ASRock B650M Pro RS"), "ASRock B650M Pro RS");
        assert_eq!(board_name("Acme Boards Inc.", "X1"), "Acme Boards X1");
        assert_eq!(board_name("", "X1"), "X1");
    }

    fn gpu(luid: &str, name: &str, vram: u64) -> GpuStatic {
        GpuStatic { luid: luid.into(), name: name.into(), vram_total: vram, ..Default::default() }
    }

    #[test]
    fn same_card_listed_three_times_is_one_card() {
        let v = vec![
            gpu("a", "RTX 4070", 0),
            gpu("b", "RTX 4070", 12),
            gpu("c", "RTX 4070", 0),
            gpu("d", "Radeon Graphics", 1),
        ];
        let r = merge_same_card(v, |l| Some(if l == "d" { (1, 0, 0) } else { (2, 0, 0) }));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].luid, "b");
        assert_eq!(r[0].alias_luids.len(), 2);
    }

    #[test]
    fn two_real_cards_at_different_addresses_stay_two() {
        let v = vec![gpu("a", "RTX 4070", 12), gpu("b", "RTX 4070", 12)];
        let r = merge_same_card(v, |l| Some(if l == "a" { (1, 0, 0) } else { (2, 0, 0) }));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn unknown_address_never_merges() {
        let v = vec![gpu("a", "RTX 4070", 12), gpu("b", "RTX 4070", 12)];
        assert_eq!(merge_same_card(v, |_| None).len(), 2);
    }

    #[test]
    fn virtual_display_adapter_without_pci_address_folds_into_the_real_card() {
        let v = vec![
            gpu("virt", "RTX 4070", 12),
            gpu("real", "RTX 4070", 12),
            gpu("amd", "Radeon Graphics", 1),
        ];
        let r = merge_same_card(v, |l| match l {
            "virt" => Some((u32::MAX, 0xFFFF, 0xFFFF)),
            "real" => Some((1, 0, 0)),
            _ => Some((16, 0, 0)),
        });
        assert_eq!(r.len(), 2);
        let card = r.iter().find(|g| g.name == "RTX 4070").unwrap();
        assert_eq!(card.luid, "real");
        assert_eq!(card.alias_luids, vec!["virt".to_string()]);
    }

    #[test]
    fn virtual_adapter_alone_is_still_listed() {
        let v = vec![gpu("virt", "Virtual GPU", 0)];
        assert_eq!(merge_same_card(v, |_| Some((u32::MAX, 0xFFFF, 0xFFFF))).len(), 1);
    }

    #[test]
    fn second_virtual_adapter_with_device_ffff_also_folds_in() {
        let v = vec![gpu("real", "RTX 4070", 12), gpu("v1", "RTX 4070", 12), gpu("v2", "RTX 4070", 12)];
        let r = merge_same_card(v, |l| match l {
            "real" => Some((1, 0, 0)),
            "v1" => Some((u32::MAX, 0xFFFF, 0xFFFF)),
            _ => Some((0, 0xFFFF, 0xFFFF)),
        });
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].luid, "real");
        assert_eq!(r[0].alias_luids.len(), 2);
    }
}
