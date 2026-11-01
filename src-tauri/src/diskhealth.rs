#![allow(unsafe_op_in_unsafe_fn)]
//! Disk health without admin rights.
//!
//! * Temperature: `IOCTL_STORAGE_QUERY_PROPERTY` / StorageDeviceTemperatureProperty (NVMe and most SATA drives;
//!   USB bridges usually don't answer). Includes the drive's own warning / critical thresholds.
//! * NVMe: the SMART / health information log page (`StorageDeviceProtocolSpecificProperty`, log page 2) —
//!   what Windows' own "Drive health" uses: critical warning bits, wear (percentage used), available spare,
//!   data written, power-on hours, unsafe shutdowns, media errors.
//!
//! The device is opened with no access rights (enough for these queries), so it works for a normal user.

use serde::Serialize;
use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
const STORAGE_DEVICE_TEMPERATURE_PROPERTY: u32 = 52;
const STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY: u32 = 50;
const PROTOCOL_TYPE_NVME: u32 = 3;
const NVME_DATA_TYPE_LOG_PAGE: u32 = 2;
const NVME_LOG_PAGE_HEALTH_INFO: u32 = 2;
/// STORAGE_PROPERTY_QUERY header (PropertyId, QueryType) + STORAGE_PROTOCOL_SPECIFIC_DATA (40 bytes).
const PROTOCOL_HEADER: usize = 8 + 40;
const HEALTH_LOG_LEN: usize = 512;
/// Highest \\.\PhysicalDriveN probed.
const MAX_DRIVES: u32 = 16;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NvmeHealth {
    /// Critical warning bits (spare low, temperature, reliability degraded, read-only, backup failed).
    pub critical_warning: u8,
    pub spare_pct: u8,
    pub spare_threshold_pct: u8,
    /// Wear: the vendor's estimate of life used (may exceed 100).
    pub used_pct: u8,
    pub read_bytes: f64,
    pub written_bytes: f64,
    pub power_cycles: u64,
    pub power_on_hours: u64,
    pub unsafe_shutdowns: u64,
    pub media_errors: u64,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiskHealth {
    /// \\.\PhysicalDriveN (matches `DriveStatic::index`).
    pub index: u32,
    pub temp_c: Option<i16>,
    pub warn_c: Option<i16>,
    pub crit_c: Option<i16>,
    pub nvme: Option<NvmeHealth>,
    /// "good", "warn", "bad", or "" when only the temperature is known.
    pub status: String,
}

fn le(bytes: &[u8]) -> u128 {
    bytes.iter().rev().fold(0u128, |acc, b| (acc << 8) | *b as u128)
}

/// STORAGE_TEMPERATURE_DATA_DESCRIPTOR -> (current, warning threshold, critical threshold), all °C.
/// The current value is the first valid sensor: on NVMe that is the composite temperature, the one drive tools
/// (CrystalDiskInfo, Windows' "Drive health") show; the other entries are individual sensors.
pub fn parse_temperature(buf: &[u8]) -> Option<(i16, Option<i16>, Option<i16>)> {
    if buf.len() < 24 {
        return None;
    }
    let critical = i16::from_le_bytes([buf[8], buf[9]]);
    let warning = i16::from_le_bytes([buf[10], buf[11]]);
    let count = u16::from_le_bytes([buf[12], buf[13]]) as usize;
    let current = (0..count)
        .map(|i| 24 + 16 * i)
        .take_while(|off| off + 4 <= buf.len())
        .map(|off| i16::from_le_bytes([buf[off + 2], buf[off + 3]]))
        .find(|t| (-60..=150).contains(t) && *t != 0)?;
    let threshold = |v: i16| (v > 0 && v < 200).then_some(v);
    Some((current, threshold(warning), threshold(critical)))
}

/// NVME_HEALTH_INFO_LOG (512 bytes) -> the fields shown. Data units are 1000 × 512 bytes.
pub fn parse_nvme_log(log: &[u8]) -> Option<(NvmeHealth, Option<i16>)> {
    if log.len() < 176 {
        return None;
    }
    let kelvin = u16::from_le_bytes([log[1], log[2]]);
    let temp = (kelvin > 200 && kelvin < 450).then(|| kelvin as i16 - 273);
    let units = |r: std::ops::Range<usize>| le(&log[r]) as f64 * 512_000.0;
    let count = |r: std::ops::Range<usize>| le(&log[r]).min(u64::MAX as u128) as u64;
    Some((
        NvmeHealth {
            critical_warning: log[0],
            spare_pct: log[3],
            spare_threshold_pct: log[4],
            used_pct: log[5],
            read_bytes: units(32..48),
            written_bytes: units(48..64),
            power_cycles: count(112..128),
            power_on_hours: count(128..144),
            unsafe_shutdowns: count(144..160),
            media_errors: count(160..176),
        },
        temp,
    ))
}

/// Overall verdict, the way drive tools phrase it: "bad" when the drive itself raises a critical warning or is
/// worn out, "warn" when it is getting there (≥ 80 % wear, spare near its threshold, media errors, too hot).
pub fn verdict(h: &DiskHealth) -> &'static str {
    let hot = matches!((h.temp_c, h.crit_c), (Some(t), Some(c)) if t >= c);
    let warm = matches!((h.temp_c, h.warn_c), (Some(t), Some(w)) if t >= w);
    match &h.nvme {
        Some(n) => {
            if n.critical_warning != 0 || n.used_pct >= 100 || hot {
                "bad"
            } else if n.used_pct >= 80
                || n.media_errors > 0
                || (n.spare_threshold_pct > 0 && n.spare_pct < n.spare_threshold_pct.saturating_add(10))
                || warm
            {
                "warn"
            } else {
                "good"
            }
        }
        None if hot => "bad",
        None if warm => "warn",
        None => "",
    }
}

unsafe fn query(h: HANDLE, input: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; out_len.max(input.len())];
    buf[..input.len()].copy_from_slice(input);
    let mut returned = 0u32;
    DeviceIoControl(
        h,
        IOCTL_STORAGE_QUERY_PROPERTY,
        Some(buf.as_ptr() as *const c_void),
        input.len() as u32,
        Some(buf.as_mut_ptr() as *mut c_void),
        buf.len() as u32,
        Some(&mut returned),
        None,
    )
    .ok()?;
    buf.truncate(returned as usize);
    Some(buf)
}

unsafe fn read_handle(h: HANDLE, index: u32) -> Option<DiskHealth> {
    let mut out = DiskHealth { index, ..Default::default() };
    let mut q = Vec::with_capacity(12);
    q.extend_from_slice(&STORAGE_DEVICE_TEMPERATURE_PROPERTY.to_le_bytes());
    q.extend_from_slice(&0u32.to_le_bytes()); // PropertyStandardQuery
    q.extend_from_slice(&[0u8; 4]);
    if let Some((t, w, c)) = query(h, &q, 1024).as_deref().and_then(parse_temperature) {
        out.temp_c = Some(t);
        out.warn_c = w;
        out.crit_c = c;
    }

    let mut p = Vec::with_capacity(PROTOCOL_HEADER + HEALTH_LOG_LEN);
    p.extend_from_slice(&STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    for v in [PROTOCOL_TYPE_NVME, NVME_DATA_TYPE_LOG_PAGE, NVME_LOG_PAGE_HEALTH_INFO, 0, 40, HEALTH_LOG_LEN as u32, 0, 0] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.extend_from_slice(&[0u8; 8]);
    p.resize(PROTOCOL_HEADER + HEALTH_LOG_LEN, 0);
    if let Some(buf) = query(h, &p, p.len()) {
        if buf.len() >= PROTOCOL_HEADER + HEALTH_LOG_LEN {
            if let Some((n, t)) = parse_nvme_log(&buf[PROTOCOL_HEADER..PROTOCOL_HEADER + HEALTH_LOG_LEN]) {
                if out.temp_c.is_none() {
                    out.temp_c = t;
                }
                out.nvme = Some(n);
            }
        }
    }
    if out.temp_c.is_none() && out.nvme.is_none() {
        return None;
    }
    out.status = verdict(&out).to_string();
    Some(out)
}

/// Every physical drive that reports a temperature or NVMe health (others are left out).
pub fn read_all() -> Vec<DiskHealth> {
    let mut out = Vec::new();
    for index in 0..MAX_DRIVES {
        let path: Vec<u16> = format!("\\\\.\\PhysicalDrive{index}").encode_utf16().chain(Some(0)).collect();
        unsafe {
            let Ok(h) = CreateFileW(
                PCWSTR(path.as_ptr()),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) else {
                continue;
            };
            if let Some(h_info) = read_handle(h, index) {
                out.push(h_info);
            }
            let _ = CloseHandle(h);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// StorageDeviceTemperatureProperty as returned by the Samsung 990 PRO on the dev machine (trimmed to 3 infos).
    fn temp_sample() -> Vec<u8> {
        let mut b = hex("280000002800000055005200030000000000000000000000");
        for (i, t) in [(0u16, 47i16), (1, 47), (2, 54)] {
            let mut info = vec![0u8; 16];
            info[0..2].copy_from_slice(&i.to_le_bytes());
            info[2..4].copy_from_slice(&t.to_le_bytes());
            info[4..6].copy_from_slice(&(-274i16).to_le_bytes());
            b.extend(info);
        }
        b
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn nvme_log(critical: u8, spare: u8, thr: u8, used: u8, media: u64) -> Vec<u8> {
        let mut log = vec![0u8; 512];
        log[0] = critical;
        log[1..3].copy_from_slice(&(46u16 + 273).to_le_bytes());
        log[3] = spare;
        log[4] = thr;
        log[5] = used;
        log[48..56].copy_from_slice(&105_312_500u64.to_le_bytes()); // data units written
        log[112..120].copy_from_slice(&1330u64.to_le_bytes());
        log[128..136].copy_from_slice(&7035u64.to_le_bytes());
        log[144..152].copy_from_slice(&160u64.to_le_bytes());
        log[160..168].copy_from_slice(&media.to_le_bytes());
        log
    }

    #[test]
    fn temperature_is_the_composite_sensor_with_the_thresholds() {
        assert_eq!(parse_temperature(&temp_sample()), Some((47, Some(82), Some(85))));
        assert_eq!(parse_temperature(&[0u8; 10]), None);
        // a drive that reports nothing usable
        let mut empty = temp_sample();
        for i in 0..3 {
            empty[24 + 16 * i + 2] = 0;
            empty[24 + 16 * i + 3] = 0;
        }
        assert_eq!(parse_temperature(&empty), None);
    }

    #[test]
    fn nvme_log_fields() {
        let (n, t) = parse_nvme_log(&nvme_log(0, 100, 10, 2, 0)).unwrap();
        assert_eq!(t, Some(46));
        assert_eq!((n.spare_pct, n.spare_threshold_pct, n.used_pct), (100, 10, 2));
        assert_eq!((n.power_cycles, n.power_on_hours, n.unsafe_shutdowns, n.media_errors), (1330, 7035, 160, 0));
        assert!((n.written_bytes / 1e12 - 53.92).abs() < 0.01, "{}", n.written_bytes);
    }

    #[test]
    fn verdicts() {
        let with = |log: Vec<u8>, temp: Option<i16>| {
            let (n, _) = parse_nvme_log(&log).unwrap();
            DiskHealth { index: 0, temp_c: temp, warn_c: Some(82), crit_c: Some(85), nvme: Some(n), status: String::new() }
        };
        assert_eq!(verdict(&with(nvme_log(0, 100, 10, 2, 0), Some(46))), "good");
        assert_eq!(verdict(&with(nvme_log(0, 100, 10, 85, 0), Some(46))), "warn", "worn");
        assert_eq!(verdict(&with(nvme_log(0, 100, 10, 2, 3), Some(46))), "warn", "media errors");
        assert_eq!(verdict(&with(nvme_log(0, 15, 10, 2, 0), Some(46))), "warn", "spare near threshold");
        assert_eq!(verdict(&with(nvme_log(0, 100, 10, 2, 0), Some(83))), "warn", "above the warning temperature");
        assert_eq!(verdict(&with(nvme_log(0x04, 100, 10, 2, 0), Some(46))), "bad", "reliability degraded");
        assert_eq!(verdict(&with(nvme_log(0, 100, 10, 100, 0), Some(46))), "bad", "worn out");
        let sata = DiskHealth { temp_c: Some(40), warn_c: Some(60), crit_c: Some(70), ..Default::default() };
        assert_eq!(verdict(&sata), "", "temperature alone says nothing about wear");
    }

    #[test]
    #[ignore = "reads the real drives of this machine"]
    fn live_read() {
        for d in read_all() {
            println!("{d:?}");
        }
    }
}
