#![allow(unsafe_op_in_unsafe_fn)]
//! Wi-Fi through the WLAN API: the current connection (name, signal quality, link rates), or, when the adapter
//! isn't connected, its state and the networks it last saw (Windows' own background scan, nothing is scanned
//! here). Returns None on machines without a Wi-Fi adapter.

use serde::Serialize;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    dot11_radio_state_off, wlan_interface_state_connected, wlan_intf_opcode_current_connection,
    wlan_intf_opcode_radio_state, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
    WlanOpenHandle, WlanQueryInterface, WLAN_AVAILABLE_NETWORK, WLAN_AVAILABLE_NETWORK_LIST,
    WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_RADIO_STATE,
};

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WifiInfo {
    pub connected: bool,
    /// The adapter's name ("Intel(R) Wi-Fi 6 AX200 160MHz").
    pub adapter: String,
    /// Not connected because the radio is switched off (Windows' Wi-Fi switch, airplane mode or a hardware key).
    pub radio_off: bool,
    pub ssid: String,
    /// 0..100, as reported by the driver.
    pub signal: u32,
    pub rx_mbps: u32,
    pub tx_mbps: u32,
    pub standard: String,
    /// Not connected: the networks in reach (by name) and the strongest of them.
    pub networks: u32,
    pub best_ssid: String,
    pub best_signal: u32,
}

pub fn read() -> Option<WifiInfo> {
    unsafe {
        let (mut version, mut handle) = (0u32, HANDLE::default());
        if WlanOpenHandle(2, None, &mut version, &mut handle) != 0 {
            return None;
        }
        let out = query(handle);
        WlanCloseHandle(handle, None);
        out
    }
}

fn wide_str(w: &[u16]) -> String {
    let len = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..len]).trim().to_string()
}

fn ssid_of(len: u32, bytes: &[u8; 32]) -> String {
    String::from_utf8_lossy(&bytes[..(len as usize).min(32)]).to_string()
}

fn standard(phy: i32) -> &'static str {
    match phy {
        4 => "802.11a",
        5 => "802.11b",
        6 => "802.11g",
        7 => "Wi-Fi 4 (802.11n)",
        8 => "Wi-Fi 5 (802.11ac)",
        9 => "802.11ad",
        10 => "Wi-Fi 6 (802.11ax)",
        11 => "Wi-Fi 7 (802.11be)",
        _ => "",
    }
}

/// One adapter's connection, or None when it isn't connected.
unsafe fn connection(handle: HANDLE, info: &WLAN_INTERFACE_INFO) -> Option<WifiInfo> {
    if info.isState != wlan_interface_state_connected {
        return None;
    }
    let (mut size, mut data): (u32, *mut c_void) = (0, null_mut());
    let st = WlanQueryInterface(handle, &info.InterfaceGuid, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None);
    if st != 0 || data.is_null() {
        return None;
    }
    let a = &(*(data as *const WLAN_CONNECTION_ATTRIBUTES)).wlanAssociationAttributes;
    let out = WifiInfo {
        connected: true,
        adapter: wide_str(&info.strInterfaceDescription),
        ssid: ssid_of(a.dot11Ssid.uSSIDLength, &a.dot11Ssid.ucSSID),
        signal: a.wlanSignalQuality.min(100),
        rx_mbps: a.ulRxRate / 1000,
        tx_mbps: a.ulTxRate / 1000,
        standard: standard(a.dot11PhyType.0).to_string(),
        ..Default::default()
    };
    WlanFreeMemory(data);
    Some(out)
}

/// The radio is off when every radio of the adapter is off in software or hardware.
unsafe fn radio_off(handle: HANDLE, info: &WLAN_INTERFACE_INFO) -> bool {
    let (mut size, mut data): (u32, *mut c_void) = (0, null_mut());
    let st = WlanQueryInterface(handle, &info.InterfaceGuid, wlan_intf_opcode_radio_state, None, &mut size, &mut data, None);
    if st != 0 || data.is_null() {
        return false;
    }
    let r = &*(data as *const WLAN_RADIO_STATE);
    let n = (r.dwNumberOfPhys as usize).min(r.PhyRadioState.len());
    let off = n > 0
        && r.PhyRadioState[..n]
            .iter()
            .all(|p| p.dot11SoftwareRadioState == dot11_radio_state_off || p.dot11HardwareRadioState == dot11_radio_state_off);
    WlanFreeMemory(data);
    off
}

/// Networks in reach by name (hidden ones and the same name on several bands count once), and the strongest.
pub fn summarize(networks: &[(String, u32)]) -> (u32, String, u32) {
    let mut seen: Vec<&str> = Vec::new();
    let (mut best, mut best_signal) = (String::new(), 0u32);
    for (ssid, signal) in networks {
        if ssid.is_empty() {
            continue;
        }
        if !seen.contains(&ssid.as_str()) {
            seen.push(ssid);
        }
        if *signal > best_signal {
            best_signal = *signal;
            best = ssid.clone();
        }
    }
    (seen.len() as u32, best, best_signal.min(100))
}

unsafe fn nearby(handle: HANDLE, info: &WLAN_INTERFACE_INFO) -> Vec<(String, u32)> {
    let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = null_mut();
    if WlanGetAvailableNetworkList(handle, &info.InterfaceGuid, 0, None, &mut list) != 0 || list.is_null() {
        return Vec::new();
    }
    let items: &[WLAN_AVAILABLE_NETWORK] =
        std::slice::from_raw_parts((*list).Network.as_ptr(), (*list).dwNumberOfItems as usize);
    let out = items.iter().map(|n| (ssid_of(n.dot11Ssid.uSSIDLength, &n.dot11Ssid.ucSSID), n.wlanSignalQuality)).collect();
    WlanFreeMemory(list as *const c_void);
    out
}

unsafe fn query(handle: HANDLE) -> Option<WifiInfo> {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    if WlanEnumInterfaces(handle, None, &mut list) != 0 || list.is_null() {
        return None;
    }
    let infos = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), (*list).dwNumberOfItems as usize);
    // A connected adapter wins; otherwise the first one, with what it can see.
    let out = infos.iter().find_map(|i| connection(handle, i)).or_else(|| {
        infos.first().map(|i| {
            let off = radio_off(handle, i);
            let (networks, best_ssid, best_signal) = if off { (0, String::new(), 0) } else { summarize(&nearby(handle, i)) };
            WifiInfo { adapter: wide_str(&i.strInterfaceDescription), radio_off: off, networks, best_ssid, best_signal, ..Default::default() }
        })
    });
    WlanFreeMemory(list as *const c_void);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_networks_count_by_name_and_name_the_strongest() {
        let list = vec![
            ("Ev_5G".to_string(), 82),
            ("Ev".to_string(), 64),
            ("Ev_5G".to_string(), 90), // the same network on a second access point / band
            (String::new(), 99),       // hidden network: no name to show
            ("Komsu".to_string(), 31),
        ];
        assert_eq!(summarize(&list), (3, "Ev_5G".to_string(), 90));
        assert_eq!(summarize(&[]), (0, String::new(), 0));
    }

    #[test]
    #[ignore = "needs a Wi-Fi adapter: run with --ignored --nocapture"]
    fn live_wifi() {
        println!("{:?}", read());
    }
}
