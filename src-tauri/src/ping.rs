#![allow(unsafe_op_in_unsafe_fn)]
//! ICMP latency to user-defined targets (Windows IcmpSendEcho, no admin needed).
//! The special target "gateway" resolves to the current default gateway.

use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Win32::NetworkManagement::IpHelper::{
    GetBestRoute, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, MIB_IPFORWARDROW,
};

pub const GATEWAY: &str = "gateway";
const TIMEOUT_MS: u32 = 1000;
const DNS_TTL: Duration = Duration::from_secs(300);
/// A failed lookup is retried soon: the network may simply have been down a moment ago.
const DNS_NEG_TTL: Duration = Duration::from_secs(10);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PingSample {
    pub target: String,
    pub ip: Option<String>,
    /// Round trip in milliseconds; None = timeout / unreachable.
    pub rtt: Option<u32>,
}

static DNS: Mutex<Option<HashMap<String, (Option<Ipv4Addr>, Instant)>>> = Mutex::new(None);

fn default_gateway() -> Option<Ipv4Addr> {
    let mut row = unsafe { std::mem::zeroed::<MIB_IPFORWARDROW>() };
    // Ask the routing table how it would reach a public address; its next hop is the gateway.
    let probe = u32::from_ne_bytes([8, 8, 8, 8]);
    if unsafe { GetBestRoute(probe, None, &mut row) } != 0 || row.dwForwardNextHop == 0 {
        return None;
    }
    Some(Ipv4Addr::from(row.dwForwardNextHop.to_ne_bytes()))
}

fn resolve(target: &str) -> Option<Ipv4Addr> {
    if target == GATEWAY {
        return default_gateway();
    }
    if let Ok(ip) = target.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    let mut guard = DNS.lock().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some((ip, at)) = cache.get(target) {
        if at.elapsed() < if ip.is_some() { DNS_TTL } else { DNS_NEG_TTL } {
            return *ip;
        }
    }
    drop(guard);
    let ip = (target, 0u16)
        .to_socket_addrs()
        .ok()
        .and_then(|mut it| it.find_map(|a| if let std::net::SocketAddr::V4(v4) = a { Some(*v4.ip()) } else { None }));
    DNS.lock().unwrap().get_or_insert_with(HashMap::new).insert(target.to_string(), (ip, Instant::now()));
    ip
}

fn echo(ip: Ipv4Addr) -> Option<u32> {
    unsafe {
        let Ok(handle) = IcmpCreateFile() else {
            return None;
        };
        let payload = [0x61u8; 32];
        // u64 backing keeps the reply struct aligned; room for the echo data plus an ICMP error.
        let mut buf = vec![0u64; (std::mem::size_of::<ICMP_ECHO_REPLY>() + payload.len() + 64).div_ceil(8)];
        let n = IcmpSendEcho(
            handle,
            u32::from_ne_bytes(ip.octets()),
            payload.as_ptr() as *const c_void,
            payload.len() as u16,
            None,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 8) as u32,
            TIMEOUT_MS,
        );
        let rtt = if n > 0 {
            let reply = &*(buf.as_ptr() as *const ICMP_ECHO_REPLY);
            (reply.Status == 0).then_some(reply.RoundTripTime)
        } else {
            None
        };
        let _ = IcmpCloseHandle(handle);
        rtt
    }
}

/// Ping every target in parallel (a dead target must not delay the others).
pub fn ping_all(targets: &[String]) -> Vec<PingSample> {
    std::thread::scope(|s| {
        let jobs: Vec<_> = targets
            .iter()
            .map(|t| {
                s.spawn(move || {
                    let ip = resolve(t);
                    PingSample {
                        target: t.clone(),
                        ip: ip.map(|i| i.to_string()),
                        rtt: ip.and_then(echo),
                    }
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs a network / Wi-Fi adapter: run with --ignored --nocapture"]
    fn live_ping() {
        let r = super::ping_all(&["127.0.0.1".into(), super::GATEWAY.into(), "1.1.1.1".into(), "8.8.8.8".into(), "dns.google".into(), "no-such-host.invalid".into()]);
        for s in &r {
            println!("{:<24} {:<16?} {:?}", s.target, s.ip, s.rtt);
        }
        assert_eq!(r[0].rtt.is_some(), true, "loopback must answer");
    }
}
