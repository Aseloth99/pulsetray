//! Battery history from Windows' own battery report (`powercfg /batteryreport /xml`).
//!
//! Windows keeps a long record of every battery: full-charge capacity per day (per week further back), cycle
//! counts, how long the PC ran on battery and on AC, and the charge level over the last days. Its HTML report is
//! hard to read, so PulseTray asks for the XML version and draws it itself. No admin rights are needed. On a PC
//! without a battery powercfg fails, and the UI says so.
//!
//! The XML's exact spelling varies between Windows builds (attribute vs. child element, "LocalTimestamp" vs.
//! "LocalTimeStamp"), so values are looked up case-insensitively in both places.

use quick_xml::events::Event;
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct Node {
    /// Local name, lower case.
    name: String,
    /// (lower-case name, value)
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The first element of this name anywhere below.
    fn find(&self, name: &str) -> Option<&Node> {
        for c in &self.children {
            if c.name == name {
                return Some(c);
            }
            if let Some(f) = c.find(name) {
                return Some(f);
            }
        }
        None
    }

    /// An attribute, or else a child element's text.
    fn get(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.attrs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.trim())
            .or_else(|| self.children.iter().find(|c| c.name == name).map(|c| c.text.trim()))
            .filter(|v| !v.is_empty())
    }

    fn num(&self, name: &str) -> Option<u64> {
        let v = self.get(name)?;
        v.parse::<u64>().ok().or_else(|| v.parse::<f64>().ok().filter(|f| *f >= 0.0).map(|f| f as u64))
    }

    fn secs(&self, name: &str) -> u64 {
        self.get(name).and_then(iso_duration_secs).unwrap_or(0)
    }
}

fn lower_local(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn parse_tree(xml: &str) -> Result<Node, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<Node> = vec![Node::default()];
    let open = |e: &quick_xml::events::BytesStart| Node {
        name: lower_local(e.local_name().as_ref()),
        attrs: e
            .attributes()
            .flatten()
            .map(|a| {
                let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.to_string()).unwrap_or_default();
                (lower_local(a.key.local_name().as_ref()), value)
            })
            .collect(),
        ..Default::default()
    };
    loop {
        match reader.read_event().map_err(|e| format!("xml: {e}"))? {
            Event::Start(e) => stack.push(open(&e)),
            Event::Empty(e) => {
                let n = open(&e);
                stack.last_mut().unwrap().children.push(n);
            }
            Event::End(_) => {
                let n = stack.pop().ok_or("xml: unbalanced")?;
                stack.last_mut().ok_or("xml: unbalanced")?.children.push(n);
            }
            Event::Text(t) => stack.last_mut().unwrap().text.push_str(&t.xml10_content()),
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    let root = stack.pop().ok_or("xml: empty")?;
    if !stack.is_empty() {
        return Err("xml: unclosed element".into());
    }
    Ok(root)
}

/// "PT1H30M5S", "P2DT3H", "PT0.5S" -> seconds.
pub fn iso_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim().strip_prefix('P')?;
    let (mut total, mut num, mut in_time) = (0f64, String::new(), false);
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' | '.' | ',' => num.push(if c == ',' { '.' } else { c }),
            _ => {
                let v: f64 = num.parse().ok()?;
                num.clear();
                total += v * match (c, in_time) {
                    ('W', false) => 604_800.0,
                    ('D', false) => 86_400.0,
                    ('H', true) => 3_600.0,
                    ('M', true) => 60.0,
                    ('S', true) => 1.0,
                    _ => return None, // years / months: not used by the report
                };
            }
        }
    }
    num.is_empty().then_some(total.round() as u64)
}

#[derive(Serialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatteryEntry {
    pub name: String,
    pub manufacturer: String,
    pub chemistry: String,
    pub design_mwh: u64,
    pub full_mwh: u64,
    pub cycles: Option<u64>,
}

/// Full-charge capacity over one period (a day recently, a week further back).
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapacityPoint {
    pub start: String,
    pub end: String,
    pub design_mwh: u64,
    pub full_mwh: u64,
    pub cycles: Option<u64>,
}

/// How long the PC was on (screen on / standby) on AC and on battery in one period.
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsagePeriod {
    pub start: String,
    pub end: String,
    pub active_ac_secs: u64,
    pub standby_ac_secs: u64,
    pub active_dc_secs: u64,
    pub standby_dc_secs: u64,
}

/// The charge level at a state change (wake, sleep, plug in / out) over the last days.
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChargePoint {
    pub t: String,
    pub ac: bool,
    /// "Active", "Suspend", "ConnectedStandby", ...
    pub kind: String,
    pub charge_mwh: u64,
    pub full_mwh: u64,
}

#[derive(Serialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BatteryHistory {
    pub report_time: String,
    pub batteries: Vec<BatteryEntry>,
    pub capacity: Vec<CapacityPoint>,
    pub usage: Vec<UsagePeriod>,
    pub recent: Vec<ChargePoint>,
}

pub fn parse(xml: &str) -> Result<BatteryHistory, String> {
    let root = parse_tree(xml)?;
    let report = root.find("batteryreport").unwrap_or(&root);
    let mut out = BatteryHistory {
        report_time: report.find("reportinformation").and_then(|r| r.get("ReportTime")).unwrap_or_default().to_string(),
        ..Default::default()
    };
    if let Some(bs) = report.find("batteries") {
        for b in bs.children("battery") {
            out.batteries.push(BatteryEntry {
                name: b.get("Id").unwrap_or_default().to_string(),
                manufacturer: b.get("Manufacturer").unwrap_or_default().to_string(),
                chemistry: b.get("Chemistry").unwrap_or_default().to_string(),
                design_mwh: b.num("DesignCapacity").unwrap_or(0),
                full_mwh: b.num("FullChargeCapacity").unwrap_or(0),
                cycles: b.num("CycleCount").filter(|&c| c > 0),
            });
        }
    }
    if let Some(h) = report.find("history") {
        for e in h.children("historyentry") {
            let start = e.get("LocalStartDate").or_else(|| e.get("StartDate")).unwrap_or_default().to_string();
            let end = e.get("LocalEndDate").or_else(|| e.get("EndDate")).unwrap_or_default().to_string();
            let (design, full) = (e.num("DesignCapacity").unwrap_or(0), e.num("FullChargeCapacity").unwrap_or(0));
            if design > 0 && full > 0 {
                out.capacity.push(CapacityPoint {
                    start: start.clone(),
                    end: end.clone(),
                    design_mwh: design,
                    full_mwh: full,
                    cycles: e.num("CycleCount").filter(|&c| c > 0),
                });
            }
            let usage = UsagePeriod {
                start,
                end,
                active_ac_secs: e.secs("ActiveAcTime"),
                standby_ac_secs: e.secs("CsAcTime"),
                active_dc_secs: e.secs("ActiveDcTime"),
                standby_dc_secs: e.secs("CsDcTime"),
            };
            if usage.active_ac_secs + usage.standby_ac_secs + usage.active_dc_secs + usage.standby_dc_secs > 0 {
                out.usage.push(usage);
            }
        }
    }
    if let Some(r) = report.find("recentusage") {
        for e in r.children("usageentry") {
            let kind = e.get("EntryType").unwrap_or_default().to_string();
            if kind.eq_ignore_ascii_case("ReportGenerated") {
                continue;
            }
            let (charge, full) = (e.num("ChargeCapacity").unwrap_or(0), e.num("FullChargeCapacity").unwrap_or(0));
            let t = e.get("LocalTimestamp").or_else(|| e.get("Timestamp")).unwrap_or_default().to_string();
            if full == 0 || t.is_empty() {
                continue;
            }
            out.recent.push(ChargePoint { t, ac: matches!(e.get("Ac"), Some("1" | "true" | "True")), kind, charge_mwh: charge, full_mwh: full });
        }
    }
    if out.batteries.is_empty() && out.capacity.is_empty() && out.recent.is_empty() {
        return Err("empty".into());
    }
    Ok(out)
}

/// The report file's text: UTF-8 (with or without a BOM) or UTF-16.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
}

/// Runs powercfg (a second or two) and reads its XML. `Err("no_battery")` on a PC without a battery.
pub fn generate(days: u32) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("powercfg.exe"))
        .unwrap_or_else(|| PathBuf::from("powercfg.exe"));
    let path = std::env::temp_dir().join(format!("pulsetray-batteryreport-{}.xml", std::process::id()));
    let mut child = std::process::Command::new(exe)
        .args(["/batteryreport", "/xml", "/duration", &days.to_string(), "/output"])
        .arg(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            return Err("powercfg timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let bytes = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    if !status.success() {
        // powercfg writes its error to stderr (in the system language, but with the error code in it).
        let mut text = String::new();
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read>), child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read>)]
            .into_iter()
            .flatten()
        {
            let mut buf = Vec::new();
            let _ = { pipe }.read_to_end(&mut buf);
            text.push_str(&String::from_utf8_lossy(&buf));
        }
        // 0x10d2 "the library, drive or media pool is empty": no battery in this PC.
        return Err(if text.contains("0x10d2") { "no_battery".into() } else { text.trim().to_string() });
    }
    bytes.map(|b| decode(&b)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout of a real report (abridged), values from a 2-year-old laptop.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<BatteryReport xmlns="http://schemas.microsoft.com/battery/2012">
  <ReportInformation><ReportVersion>1</ReportVersion><ReportTime>2026-10-25T10:00:00</ReportTime></ReportInformation>
  <Batteries>
    <Battery>
      <Id>5B10W51867</Id><Manufacturer>SMP</Manufacturer><Chemistry>LiP</Chemistry>
      <DesignCapacity>57000</DesignCapacity><FullChargeCapacity>51870</FullChargeCapacity><CycleCount>214</CycleCount>
    </Battery>
  </Batteries>
  <RecentUsage>
    <UsageEntry Timestamp="2026-10-24T05:00:00Z" LocalTimestamp="2026-10-24T08:00:00" Duration="PT1H" Ac="1" EntryType="Active" ChargeCapacity="51000" FullChargeCapacity="51870" />
    <UsageEntry Timestamp="2026-10-24T09:00:00Z" LocalTimestamp="2026-10-24T12:00:00" Duration="PT2H" Ac="0" EntryType="Active" ChargeCapacity="31122" FullChargeCapacity="51870" />
    <UsageEntry LocalTimestamp="2026-10-25T10:00:00" EntryType="ReportGenerated" Ac="1" ChargeCapacity="1" FullChargeCapacity="1" />
  </RecentUsage>
  <History>
    <HistoryEntry LocalStartDate="2024-10-07T00:00:00" LocalEndDate="2024-10-14T00:00:00" ActiveAcTime="PT20H" CsAcTime="PT0S" ActiveDcTime="PT5H30M" CsDcTime="PT0S" DesignCapacity="57000" FullChargeCapacity="57000" CycleCount="3" />
    <HistoryEntry LocalStartDate="2026-10-23T00:00:00" LocalEndDate="2026-10-24T00:00:00" ActiveAcTime="PT6H" CsAcTime="PT1H" ActiveDcTime="PT2H15M" CsDcTime="P0D" DesignCapacity="57000" FullChargeCapacity="51870" CycleCount="214" />
    <HistoryEntry LocalStartDate="2026-10-24T00:00:00" LocalEndDate="2026-10-25T00:00:00" DesignCapacity="0" FullChargeCapacity="0" />
  </History>
</BatteryReport>"#;

    #[test]
    fn reads_batteries_capacity_history_usage_and_recent_charge() {
        let h = parse(SAMPLE).unwrap();
        assert_eq!(h.report_time, "2026-10-25T10:00:00");
        assert_eq!(h.batteries, vec![BatteryEntry {
            name: "5B10W51867".into(),
            manufacturer: "SMP".into(),
            chemistry: "LiP".into(),
            design_mwh: 57000,
            full_mwh: 51870,
            cycles: Some(214)
        }]);
        assert_eq!(h.capacity.len(), 2, "periods without a capacity are left out");
        assert_eq!((h.capacity[0].end.as_str(), h.capacity[0].full_mwh, h.capacity[0].cycles), ("2024-10-14T00:00:00", 57000, Some(3)));
        assert_eq!(h.usage.len(), 2);
        assert_eq!(
            (h.usage[1].active_ac_secs, h.usage[1].standby_ac_secs, h.usage[1].active_dc_secs),
            (6 * 3600, 3600, 2 * 3600 + 15 * 60)
        );
        assert_eq!(h.recent.len(), 2, "the report's own entry is left out");
        assert_eq!((h.recent[1].t.as_str(), h.recent[1].ac, h.recent[1].charge_mwh), ("2026-10-24T12:00:00", false, 31122));
    }

    #[test]
    fn other_spellings_and_child_elements_are_read_too() {
        let xml = r#"<batteryreport><batteries><battery><designcapacity>40000</designcapacity>
            <fullchargecapacity>30000</fullchargecapacity><cyclecount>0</cyclecount></battery></batteries>
            <recentusage><usageentry LocalTimeStamp="2026-10-24T08:00:00" AC="1" ChargeCapacity="15000" FullChargeCapacity="30000"/></recentusage>
            </batteryreport>"#;
        let h = parse(xml).unwrap();
        assert_eq!((h.batteries[0].design_mwh, h.batteries[0].cycles), (40000, None), "cycle count 0 = not reported");
        assert_eq!((h.recent[0].t.as_str(), h.recent[0].ac), ("2026-10-24T08:00:00", true));
    }

    #[test]
    fn broken_or_empty_reports_are_errors() {
        assert!(parse("<BatteryReport><Batteries></Batteries></BatteryReport>").is_err());
        assert!(parse("<BatteryReport><Batteries>").is_err());
        assert!(parse("not xml at all").is_err());
    }

    #[test]
    fn iso_durations() {
        assert_eq!(iso_duration_secs("PT1H30M5S"), Some(5405));
        assert_eq!(iso_duration_secs("P1DT2H"), Some(93_600));
        assert_eq!(iso_duration_secs("PT0.6S"), Some(1));
        assert_eq!(iso_duration_secs("P0D"), Some(0));
        assert_eq!(iso_duration_secs("1H"), None);
        assert_eq!(iso_duration_secs("P1Y"), None);
    }

    #[test]
    fn utf16_and_bom_files() {
        let mut b = vec![0xFF, 0xFE];
        for u in "<a/>".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode(&b), "<a/>");
        assert_eq!(decode(b"\xEF\xBB\xBF<a/>"), "<a/>");
    }

    #[test]
    #[ignore = "runs powercfg on this PC: run with --ignored --nocapture"]
    fn this_pc() {
        // The user's desktop has no battery: powercfg must come back with "no_battery", quickly.
        match generate(3) {
            Ok(xml) => println!("report: {} bytes, parsed: {:?}", xml.len(), parse(&xml).map(|h| h.batteries)),
            Err(e) => println!("no report: {e}"),
        }
    }
}
