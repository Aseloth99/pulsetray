#![allow(unsafe_op_in_unsafe_fn)]
//! Battery capacity / wear / power draw through the Windows battery device interface
//! (the same IOCTLs powercfg uses). Returns None on desktops or if the driver doesn't answer.
//! Machines with more than one battery get the packs summed, so capacity, rate and health describe
//! the whole system and match the system-wide charge percentage.

use serde::Serialize;
use std::ffi::c_void;
use std::mem::size_of;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
    GUID_DEVCLASS_BATTERY, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Power::{
    BatteryInformation, BATTERY_INFORMATION, BATTERY_QUERY_INFORMATION, BATTERY_STATUS,
    BATTERY_WAIT_STATUS, IOCTL_BATTERY_QUERY_INFORMATION, IOCTL_BATTERY_QUERY_STATUS,
    IOCTL_BATTERY_QUERY_TAG,
};
use windows::Win32::System::IO::DeviceIoControl;

const GENERIC_READ_WRITE: u32 = 0xC000_0000;
/// BATTERY_UNKNOWN_CAPACITY / BATTERY_UNKNOWN_VOLTAGE.
const UNKNOWN_U32: u32 = 0xFFFF_FFFF;
/// BATTERY_UNKNOWN_RATE (a signed value, 0x80000000).
const UNKNOWN_RATE: u32 = 0x8000_0000;
/// BATTERY_CAPACITY_RELATIVE: capacities are percentages / arbitrary units, not mWh.
const CAPACITY_RELATIVE: u32 = 0x4000_0000;
const MAX_BATTERIES: u32 = 8;

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatteryDetail {
    pub design_mwh: u32,
    pub full_mwh: u32,
    pub current_mwh: u32,
    pub cycles: u32,
    /// Positive while charging, negative while discharging (milliwatts).
    pub rate_mw: i32,
    pub voltage_mv: u32,
}

/// Raw driver values for one pack -> sane numbers (unknown markers become 0).
pub struct Raw {
    pub designed: u32,
    pub full: u32,
    pub capabilities: u32,
    pub cycles: u32,
    /// (capacity, voltage, rate) from the status IOCTL, if it answered.
    pub status: Option<(u32, u32, i32)>,
}

pub fn sanitize(r: &Raw) -> BatteryDetail {
    let known = |v: u32| if v == UNKNOWN_U32 { 0 } else { v };
    // Relative-capacity packs don't report mWh at all; leave the energy figures empty rather than lie.
    let relative = r.capabilities & CAPACITY_RELATIVE != 0;
    let (cap, volt, rate) = r.status.unwrap_or((UNKNOWN_U32, UNKNOWN_U32, UNKNOWN_RATE as i32));
    BatteryDetail {
        design_mwh: if relative { 0 } else { known(r.designed) },
        full_mwh: if relative { 0 } else { known(r.full) },
        current_mwh: if relative { 0 } else { known(cap) },
        cycles: r.cycles,
        rate_mw: if rate as u32 == UNKNOWN_RATE { 0 } else { rate },
        voltage_mv: known(volt),
    }
}

/// Add several packs together. Voltage is averaged (series packs aren't a thing in laptops).
pub fn merge(parts: &[BatteryDetail]) -> Option<BatteryDetail> {
    let first = parts.first()?;
    if parts.len() == 1 {
        return Some(first.clone());
    }
    let volts: Vec<u32> = parts.iter().map(|p| p.voltage_mv).filter(|v| *v > 0).collect();
    Some(BatteryDetail {
        design_mwh: parts.iter().fold(0u32, |a, p| a.saturating_add(p.design_mwh)),
        full_mwh: parts.iter().fold(0u32, |a, p| a.saturating_add(p.full_mwh)),
        current_mwh: parts.iter().fold(0u32, |a, p| a.saturating_add(p.current_mwh)),
        cycles: parts.iter().map(|p| p.cycles).max().unwrap_or(0),
        rate_mw: parts.iter().fold(0i32, |a, p| a.saturating_add(p.rate_mw)),
        voltage_mv: if volts.is_empty() { 0 } else { (volts.iter().map(|v| *v as u64).sum::<u64>() / volts.len() as u64) as u32 },
    })
}

enum Step {
    /// No more battery interfaces.
    End,
    /// This one could not be read; try the next.
    Skip,
    Got(BatteryDetail),
}

pub fn read() -> Option<BatteryDetail> {
    unsafe {
        let hdev = SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_BATTERY),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .ok()?;
        let mut parts = Vec::new();
        for index in 0..MAX_BATTERIES {
            match query(hdev, index) {
                Step::End => break,
                Step::Skip => {}
                Step::Got(d) => parts.push(d),
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(hdev);
        merge(&parts)
    }
}

unsafe fn query(
    hdev: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    index: u32,
) -> Step {
    let mut did = SP_DEVICE_INTERFACE_DATA {
        cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
        ..Default::default()
    };
    if SetupDiEnumDeviceInterfaces(hdev, None, &GUID_DEVCLASS_BATTERY, index, &mut did).is_err() {
        return Step::End;
    }

    let mut required = 0u32;
    let _ = SetupDiGetDeviceInterfaceDetailW(hdev, &did, None, 0, Some(&mut required), None);
    if required == 0 {
        return Step::Skip;
    }
    let mut buf = vec![0u64; (required as usize).div_ceil(8)];
    let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
    (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    if SetupDiGetDeviceInterfaceDetailW(hdev, &did, Some(detail), required, None, None).is_err() {
        return Step::Skip;
    }

    let Ok(handle) = CreateFileW(
        PCWSTR((*detail).DevicePath.as_ptr()),
        GENERIC_READ_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        None,
    ) else {
        return Step::Skip;
    };
    let out = read_from(handle);
    let _ = CloseHandle(handle);
    out.map_or(Step::Skip, Step::Got)
}

unsafe fn ioctl<I, O>(h: HANDLE, code: u32, input: &I, output: &mut O) -> bool {
    let mut returned = 0u32;
    DeviceIoControl(
        h,
        code,
        Some(input as *const I as *const c_void),
        size_of::<I>() as u32,
        Some(output as *mut O as *mut c_void),
        size_of::<O>() as u32,
        Some(&mut returned),
        None,
    )
    .is_ok()
}

unsafe fn read_from(h: HANDLE) -> Option<BatteryDetail> {
    let mut tag = 0u32;
    if !ioctl(h, IOCTL_BATTERY_QUERY_TAG, &0u32, &mut tag) || tag == 0 {
        return None;
    }

    let q = BATTERY_QUERY_INFORMATION {
        BatteryTag: tag,
        InformationLevel: BatteryInformation,
        AtRate: 0,
    };
    let mut info = BATTERY_INFORMATION::default();
    if !ioctl(h, IOCTL_BATTERY_QUERY_INFORMATION, &q, &mut info) {
        return None;
    }

    let wait = BATTERY_WAIT_STATUS { BatteryTag: tag, ..Default::default() };
    let mut st = BATTERY_STATUS::default();
    let have_status = ioctl(h, IOCTL_BATTERY_QUERY_STATUS, &wait, &mut st);

    Some(sanitize(&Raw {
        designed: info.DesignedCapacity,
        full: info.FullChargedCapacity,
        capabilities: info.Capabilities,
        cycles: info.CycleCount,
        status: have_status.then_some((st.Capacity, st.Voltage, st.Rate)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(designed: u32, full: u32, status: Option<(u32, u32, i32)>) -> Raw {
        Raw { designed, full, capabilities: 0, cycles: 10, status }
    }

    #[test]
    fn unknown_markers_never_reach_the_ui() {
        // "unknown capacity / voltage" is 0xFFFFFFFF, "unknown rate" is 0x80000000: none may show as a number.
        let d = sanitize(&raw(UNKNOWN_U32, UNKNOWN_U32, Some((UNKNOWN_U32, UNKNOWN_U32, UNKNOWN_RATE as i32))));
        assert_eq!((d.design_mwh, d.full_mwh, d.current_mwh, d.voltage_mv, d.rate_mw), (0, 0, 0, 0, 0));
        // status IOCTL not answering at all
        let d = sanitize(&raw(40_000, 34_000, None));
        assert_eq!((d.design_mwh, d.full_mwh, d.current_mwh, d.voltage_mv, d.rate_mw), (40_000, 34_000, 0, 0, 0));
    }

    #[test]
    fn normal_readings_pass_through_with_their_sign() {
        let d = sanitize(&raw(40_000, 34_000, Some((20_000, 12_300, -8_500))));
        assert_eq!((d.current_mwh, d.voltage_mv, d.rate_mw, d.cycles), (20_000, 12_300, -8_500, 10));
        let d = sanitize(&raw(40_000, 34_000, Some((20_000, 12_300, 21_700))));
        assert_eq!(d.rate_mw, 21_700);
    }

    #[test]
    fn relative_capacity_packs_report_no_energy_figures() {
        let r = Raw { designed: 100, full: 100, capabilities: CAPACITY_RELATIVE, cycles: 0, status: Some((55, 12_000, 0)) };
        let d = sanitize(&r);
        assert_eq!((d.design_mwh, d.full_mwh, d.current_mwh), (0, 0, 0));
        assert_eq!(d.voltage_mv, 12_000);
    }

    #[test]
    fn two_packs_are_summed() {
        let a = sanitize(&raw(40_000, 36_000, Some((18_000, 12_000, -5_000))));
        let b = sanitize(&Raw { cycles: 300, ..raw(20_000, 18_000, Some((9_000, 12_400, -2_500))) });
        let m = merge(&[a, b]).unwrap();
        assert_eq!((m.design_mwh, m.full_mwh, m.current_mwh), (60_000, 54_000, 27_000));
        assert_eq!((m.rate_mw, m.cycles, m.voltage_mv), (-7_500, 300, 12_200));
        assert!(merge(&[]).is_none());
    }
}
