#![allow(unsafe_op_in_unsafe_fn)]
//! CPU temperature and power from a monitoring tool that is already running: MSI Afterburner or HWiNFO.
//!
//! Windows has no driver-free, admin-free way to read the CPU die sensor (the ACPI thermal zone is a board
//! constant on most desktops) and the energy meter exists on Intel only. Both tools read them with their own
//! driver and publish the readings in shared memory for other programs, so PulseTray only has to read that
//! memory: no driver, no admin rights, and nothing happens when neither tool runs. HWiNFO needs "Shared Memory
//! Support" switched on in its settings.

use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION,
};

/// What one tool says about the CPU.
#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub struct Reading {
    pub temp: Option<f32>,
    pub power: Option<f32>,
}

/// Plausible CPU temperature in °C (anything else is a dead sensor or garbage).
fn plausible_temp(t: f32) -> Option<f32> {
    (t.is_finite() && (10.0..130.0).contains(&t)).then_some(t)
}

/// Plausible CPU package power in W.
fn plausible_power(w: f32) -> Option<f32> {
    (w.is_finite() && (0.5..700.0).contains(&w)).then_some(w)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn f32_at(b: &[u8], o: usize) -> Option<f32> {
    Some(f32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn f64_at(b: &[u8], o: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// A zero-terminated ANSI string in a fixed-size field, lower-cased.
fn text_at(b: &[u8], o: usize, len: usize) -> Option<String> {
    let field = b.get(o..o.checked_add(len)?)?;
    let end = field.iter().position(|&c| c == 0).unwrap_or(len);
    Some(String::from_utf8_lossy(&field[..end]).to_lowercase())
}

// ---------------------------------------------------------------- MSI Afterburner ("MAHMSharedMemory")

const MAHM_SIGNATURES: [u32; 2] = [0x4D41_484D, 0x4D48_414D]; // 'MAHM' (the other order, in case); a closed tool writes 0xDEAD
const MAHM_NAME_LEN: usize = 260; // MAX_PATH
/// An entry is five MAX_PATH strings (name, unit, localized name, localized unit, format), then the value.
const MAHM_DATA: usize = 5 * MAHM_NAME_LEN;

/// Temperature: the hottest "CPU temperature" / "CPU1 temperature" ... entry; power: "CPU power" (Afterburner's
/// hardware monitor names).
pub fn parse_afterburner(b: &[u8]) -> Reading {
    let mut out = Reading::default();
    if !u32_at(b, 0).is_some_and(|s| MAHM_SIGNATURES.contains(&s)) {
        return out;
    }
    let (Some(header), Some(count), Some(size)) = (u32_at(b, 8), u32_at(b, 12), u32_at(b, 16)) else { return out };
    let (header, count, size) = (header as usize, count as usize, size as usize);
    if size < MAHM_DATA + 4 || count > 4096 {
        return out;
    }
    for i in 0..count {
        let Some(at) = i.checked_mul(size).and_then(|o| header.checked_add(o)) else { break };
        let (Some(name), Some(value)) = (text_at(b, at, MAHM_NAME_LEN), f32_at(b, at + MAHM_DATA)) else { break };
        if name.starts_with("cpu") && name.ends_with(" temperature") {
            out.temp = plausible_temp(value).into_iter().chain(out.temp).reduce(f32::max);
        } else if name == "cpu power" {
            out.power = plausible_power(value).or(out.power);
        }
    }
    out
}

// ---------------------------------------------------------------- HWiNFO ("Global\HWiNFO_SENS_SM2")

const HWINFO_SIGNATURE: u32 = 0x5369_5748; // "HWiS"; a closed HWiNFO writes "DEAD"
const HWINFO_TEMP: u32 = 1; // SENSOR_TYPE_TEMP
const HWINFO_POWER: u32 = 5; // SENSOR_TYPE_POWER
/// Reading element: type, sensor index, reading id (3 x u32), two 128-byte labels, a 16-byte unit, then the value (f64).
const HWINFO_LABEL: usize = 12;
const HWINFO_LABEL_LEN: usize = 128;
const HWINFO_VALUE: usize = 284;

/// Label substrings of the CPU die temperature, best first (Intel package, AMD Tctl/Tdie, ...).
const HWINFO_TEMP_LABELS: [&str; 5] = ["cpu package", "tctl/tdie", "cpu die (average)", "cpu tctl", "cpu die"];
/// Label substrings of the CPU package power, best first.
const HWINFO_POWER_LABELS: [&str; 2] = ["cpu package power", "cpu ppt"];

/// The best-labelled reading of one type: lowest rank in `labels` wins.
fn hwinfo_best(b: &[u8], (offset, size, count): (usize, usize, usize), kind: u32, labels: &[&str], ok: fn(f32) -> Option<f32>) -> Option<f32> {
    (0..count)
        .filter_map(|i| {
            let at = offset.checked_add(i.checked_mul(size)?)?;
            if u32_at(b, at)? != kind {
                return None;
            }
            let label = text_at(b, at + HWINFO_LABEL, HWINFO_LABEL_LEN)?;
            let rank = labels.iter().position(|k| label.contains(k))?;
            Some((rank, ok(f64_at(b, at + HWINFO_VALUE)? as f32)?))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, v)| v)
}

pub fn parse_hwinfo(b: &[u8]) -> Reading {
    if u32_at(b, 0) != Some(HWINFO_SIGNATURE) {
        return Reading::default();
    }
    let (Some(offset), Some(size), Some(count)) = (u32_at(b, 32), u32_at(b, 36), u32_at(b, 40)) else { return Reading::default() };
    let table = (offset as usize, size as usize, count as usize);
    if table.1 < HWINFO_VALUE + 8 || table.2 > 65_536 {
        return Reading::default();
    }
    Reading {
        temp: hwinfo_best(b, table, HWINFO_TEMP, &HWINFO_TEMP_LABELS, plausible_temp),
        power: hwinfo_best(b, table, HWINFO_POWER, &HWINFO_POWER_LABELS, plausible_power),
    }
}

// ---------------------------------------------------------------- reading the shared memory

/// Map a named shared-memory section read-only, hand its bytes to `parse`, unmap again. The section outlives
/// the tool that made it as long as someone holds it open, so it is never kept: stale numbers would stay.
fn with_mapping(name: &str, parse: fn(&[u8]) -> Reading) -> Option<Reading> {
    unsafe {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(wide.as_ptr())).ok()?;
        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        let mut result = None;
        if !view.Value.is_null() {
            let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            if VirtualQuery(Some(view.Value), &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) != 0 {
                result = Some(parse(std::slice::from_raw_parts(view.Value as *const u8, info.RegionSize)));
            }
            let _ = UnmapViewOfFile(view);
        }
        let _ = CloseHandle(handle);
        result
    }
}

/// The CPU readings with the tool each came from (value, source name).
#[derive(Default, Debug, PartialEq)]
pub struct Cpu {
    pub temp: Option<(f32, &'static str)>,
    pub power: Option<(f32, &'static str)>,
}

/// Afterburner first; HWiNFO only for what Afterburner did not give (and only opened then).
pub fn cpu() -> Cpu {
    let ab = with_mapping("MAHMSharedMemory", parse_afterburner).unwrap_or_default();
    let hw = if ab.temp.is_some() && ab.power.is_some() {
        Reading::default()
    } else {
        with_mapping("Global\\HWiNFO_SENS_SM2", parse_hwinfo).unwrap_or_default()
    };
    Cpu {
        temp: ab.temp.map(|t| (t, "afterburner")).or(hw.temp.map(|t| (t, "hwinfo"))),
        power: ab.power.map(|w| (w, "afterburner")).or(hw.power.map(|w| (w, "hwinfo"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn text(b: &mut [u8], o: usize, s: &str) {
        b[o..o + s.len()].copy_from_slice(s.as_bytes());
    }

    /// An Afterburner block: 40-byte header (64-bit time_t) and entries of 1316 bytes, like the real ones.
    fn afterburner(entries: &[(&str, f32)]) -> Vec<u8> {
        let size = MAHM_DATA + 16;
        let mut b = vec![0u8; 40 + entries.len() * size];
        put32(&mut b, 0, MAHM_SIGNATURES[0]);
        put32(&mut b, 4, 0x0002_0001);
        put32(&mut b, 8, 40);
        put32(&mut b, 12, entries.len() as u32);
        put32(&mut b, 16, size as u32);
        for (i, (name, v)) in entries.iter().enumerate() {
            let at = 40 + i * size;
            text(&mut b, at, name);
            b[at + MAHM_DATA..at + MAHM_DATA + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// A HWiNFO block: 44-byte header, no sensor section, reading elements of 316 bytes.
    fn hwinfo(readings: &[(u32, &str, f64)]) -> Vec<u8> {
        let size = 316;
        let mut b = vec![0u8; 44 + readings.len() * size];
        put32(&mut b, 0, HWINFO_SIGNATURE);
        put32(&mut b, 20, 44);
        put32(&mut b, 32, 44);
        put32(&mut b, 36, size as u32);
        put32(&mut b, 40, readings.len() as u32);
        for (i, (kind, label, v)) in readings.iter().enumerate() {
            let at = 44 + i * size;
            put32(&mut b, at, *kind);
            text(&mut b, at + HWINFO_LABEL, label);
            b[at + HWINFO_VALUE..at + HWINFO_VALUE + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn afterburner_cpu_readings_are_found_among_the_others() {
        let b = afterburner(&[("GPU temperature", 54.0), ("CPU usage", 91.0), ("CPU temperature", 71.0), ("CPU power", 88.1)]);
        assert_eq!(parse_afterburner(&b), Reading { temp: Some(71.0), power: Some(88.1) });
        // Several cores: the hottest.
        let b = afterburner(&[("CPU1 temperature", 60.0), ("CPU2 temperature", 66.5), ("GPU temperature", 90.0)]);
        assert_eq!(parse_afterburner(&b), Reading { temp: Some(66.5), power: None });
        assert_eq!(parse_afterburner(&afterburner(&[("GPU power", 51.5)])), Reading::default(), "only the CPU's power");
    }

    #[test]
    fn afterburner_dead_or_nonsense_readings_are_ignored() {
        assert_eq!(parse_afterburner(&afterburner(&[("CPU temperature", 0.0), ("CPU power", 0.0)])), Reading::default(), "0 = not read");
        assert_eq!(parse_afterburner(&afterburner(&[("CPU temperature", f32::NAN)])), Reading::default());
        let mut closed = afterburner(&[("CPU temperature", 71.0)]);
        put32(&mut closed, 0, 0xDEAD);
        assert_eq!(parse_afterburner(&closed), Reading::default(), "the tool was closed");
    }

    #[test]
    fn hwinfo_picks_the_cpu_readings_by_label_and_type() {
        let b = hwinfo(&[
            (1, "Core 0", 80.0),
            (5, "CPU Package Power", 88.0),
            (1, "CPU (Tctl/Tdie)", 69.5),
            (1, "CPU Die (average)", 66.0),
            (5, "GPU Power", 51.0),
            (1, "GPU Temperature", 54.0),
        ]);
        assert_eq!(parse_hwinfo(&b), Reading { temp: Some(69.5), power: Some(88.0) }, "Tctl/Tdie ranks above the die average");
        let intel = hwinfo(&[(1, "CPU Package", 58.0), (1, "CPU Die (average)", 55.0), (5, "CPU Package Power", 45.5)]);
        assert_eq!(parse_hwinfo(&intel), Reading { temp: Some(58.0), power: Some(45.5) });
        // A reading of the wrong type with a fitting label does not count.
        assert_eq!(parse_hwinfo(&hwinfo(&[(5, "CPU Package", 58.0), (1, "CPU PPT", 45.0)])), Reading::default());
        let mut closed = hwinfo(&[(1, "CPU Package", 58.0)]);
        put32(&mut closed, 0, 0x4441_4544);
        assert_eq!(parse_hwinfo(&closed), Reading::default());
    }

    #[test]
    fn truncated_or_lying_blocks_do_not_panic() {
        let good = afterburner(&[("CPU temperature", 71.0)]);
        for n in [0, 3, 19, 40, 500, 1343] {
            // 1343: the value (bytes 1340..1344) is cut in half
            assert_eq!(parse_afterburner(&good[..n]), Reading::default(), "{n} bytes");
        }
        let good = hwinfo(&[(1, "CPU Package", 58.0)]);
        for n in [0, 3, 41, 100, 335] {
            // 335: the value (bytes 328..336) is cut short
            assert_eq!(parse_hwinfo(&good[..n]), Reading::default(), "{n} bytes");
        }
        let mut huge = afterburner(&[("CPU temperature", 71.0)]);
        put32(&mut huge, 12, u32::MAX);
        assert_eq!(parse_afterburner(&huge), Reading::default());
        put32(&mut huge, 12, 1);
        put32(&mut huge, 8, u32::MAX);
        assert_eq!(parse_afterburner(&huge), Reading::default(), "a header offset past the end");
    }

    #[test]
    #[ignore = "needs Afterburner or HWiNFO running: run with --ignored --nocapture"]
    fn live_cpu_readings() {
        let c = cpu();
        println!("{c:?}");
        assert!(c.temp.is_some());
    }
}
