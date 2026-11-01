//! macik.com.tr: the remote announcement line.
//!
//! It rides on the update check (same request, no extra connection):
//!   GET /api/updates/check?appName=..&currentVersion=..
//!   reply may carry  "notice": { id, text | {tr,en}, url?, level?, expires? }

use serde::Serialize;
use serde_json::Value;

const MAX_TEXT: usize = 240;
const MAX_ID: usize = 64;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: String,
    pub text: String,
    /// Only ever an https link on macik.com.tr (empty = no link).
    pub url: String,
    /// "info" or "warn".
    pub level: String,
}

/// Drops control characters and bidirectional overrides, trims and caps the length.
fn clean(s: &str, limit: usize) -> String {
    let swapped: String = s
        .chars()
        .map(|c| match c {
            '\u{0}'..='\u{1f}' | '\u{7f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => ' ',
            c => c,
        })
        .collect();
    swapped.trim().chars().take(limit).collect()
}

fn pick(text: &Value, lang: &str) -> String {
    match text {
        Value::String(s) => clean(s, MAX_TEXT),
        Value::Object(map) => [lang, "tr", "en"]
            .iter()
            .filter_map(|k| map.get(*k).and_then(Value::as_str))
            .map(|s| clean(s, MAX_TEXT))
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2026-12-31", "2026-12-31T23:59:59Z" or with a +HH:MM offset -> unix seconds. None if malformed.
pub fn parse_expiry(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', ' ']).map_or((s, ""), |(d, r)| (d, r));
    let mut it = date.split('-');
    let (y, m, d) = (it.next()?.parse::<i64>().ok()?, it.next()?.parse::<i64>().ok()?, it.next()?.parse::<i64>().ok()?);
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1970..=9999).contains(&y) {
        return None;
    }
    let (mut secs, mut offset) = (0i64, 0i64);
    if !rest.is_empty() {
        let (clock, tz) = match rest.find(['Z', 'z', '+', '-']) {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let mut c = clock.split(':');
        let (h, mi) = (c.next()?.parse::<i64>().ok()?, c.next()?.parse::<i64>().ok()?);
        let sec = c.next().map(|x| x.split('.').next().unwrap_or("0").parse::<i64>().ok()).unwrap_or(Some(0))?;
        if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..61).contains(&sec) {
            return None;
        }
        secs = h * 3600 + mi * 60 + sec;
        if let Some(sign) = tz.chars().next().filter(|c| *c == '+' || *c == '-') {
            let body = &tz[1..];
            let (oh, om) = body.split_once(':').unwrap_or((body, "0"));
            let off = oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60;
            offset = if sign == '+' { off } else { -off };
        }
    }
    Some(days_from_civil(y, m, d) * 86_400 + secs - offset)
}

/// Validates the `notice` field of the reply. Anything invalid, empty or expired yields None, so
/// nothing is shown.
pub fn parse_notice(reply: &Value, lang: &str, now_unix: i64) -> Option<Notice> {
    let raw = reply.get("notice")?.as_object()?;
    let id = clean(raw.get("id")?.as_str()?, MAX_ID);
    let text = pick(raw.get("text")?, lang);
    if id.is_empty() || text.is_empty() {
        return None;
    }
    if let Some(exp) = raw.get("expires").and_then(Value::as_str).filter(|e| !e.trim().is_empty()) {
        if parse_expiry(exp).is_none_or(|t| t <= now_unix) {
            return None;
        }
    }
    let url = raw
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| crate::updater::validate_url(u))
        .unwrap_or_default()
        .to_string();
    let level = if raw.get("level").and_then(Value::as_str) == Some("warn") { "warn" } else { "info" };
    Some(Notice { id, text, url, level: level.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_791_936_000; // 2026-10-14T00:00:00Z

    fn reply(over: Value) -> Value {
        let mut n = json!({"id": "2026-10-a", "text": {"tr": "Merhaba", "en": "Hello"}, "url": "https://macik.com.tr/duyuru"});
        for (k, v) in over.as_object().unwrap() {
            n[k] = v.clone();
        }
        json!({"updateAvailable": false, "notice": n})
    }

    #[test]
    fn expiry_parsing() {
        assert_eq!(parse_expiry("2026-10-14T00:00:00Z"), Some(NOW));
        assert_eq!(parse_expiry("2026-10-14"), Some(NOW));
        assert_eq!(parse_expiry("2026-10-14T03:00:00+03:00"), Some(NOW));
        assert_eq!(parse_expiry("2026-12-31T00:00:00Z"), Some(1_798_675_200));
        for bad in ["", "soon", "2026-13-01", "2026-10", "2026-10-14T25:00:00Z"] {
            assert_eq!(parse_expiry(bad), None, "{bad}");
        }
    }

    #[test]
    fn language_pick_and_fallback() {
        assert_eq!(parse_notice(&reply(json!({})), "en", NOW).unwrap().text, "Hello");
        assert_eq!(parse_notice(&reply(json!({})), "tr", NOW).unwrap().text, "Merhaba");
        assert_eq!(parse_notice(&reply(json!({"text": {"en": "Only"}})), "tr", NOW).unwrap().text, "Only");
        assert_eq!(parse_notice(&reply(json!({"text": "Plain"})), "tr", NOW).unwrap().text, "Plain");
    }

    #[test]
    fn nothing_shown_without_a_valid_notice() {
        for r in [json!({}), json!({"notice": null}), json!({"notice": "x"}), reply(json!({"id": ""})), reply(json!({"text": {}})), reply(json!({"text": "  "}))] {
            assert!(parse_notice(&r, "tr", NOW).is_none());
        }
    }

    #[test]
    fn expired_notices_are_dropped() {
        assert!(parse_notice(&reply(json!({"expires": "2026-10-13T00:00:00Z"})), "tr", NOW).is_none());
        assert!(parse_notice(&reply(json!({"expires": "2026-12-31T00:00:00Z"})), "tr", NOW).is_some());
        assert!(parse_notice(&reply(json!({"expires": "not a date"})), "tr", NOW).is_none());
    }

    #[test]
    fn untrusted_links_are_dropped_but_the_text_stays() {
        for bad in ["http://macik.com.tr/a", "https://evil.example/a", "https://macik.com.tr.evil.example/a", "https://u:p@macik.com.tr/a", "javascript:alert(1)", "file:///c:/x"] {
            let n = parse_notice(&reply(json!({"url": bad})), "tr", NOW).unwrap();
            assert_eq!((n.text.as_str(), n.url.as_str()), ("Merhaba", ""), "{bad}");
        }
        assert_eq!(parse_notice(&reply(json!({"url": "https://www.macik.com.tr/x"})), "tr", NOW).unwrap().url, "https://www.macik.com.tr/x");
    }

    #[test]
    fn text_is_cleaned_and_capped() {
        let long = format!("a\u{202e}b\u{0}c\nd{}", "x".repeat(500));
        let n = parse_notice(&reply(json!({"text": long})), "tr", NOW).unwrap();
        assert!(!n.text.contains('\u{202e}') && !n.text.contains('\0') && !n.text.contains('\n'));
        assert!(n.text.chars().count() <= MAX_TEXT);
    }

    #[test]
    fn level_defaults_to_info() {
        assert_eq!(parse_notice(&reply(json!({"level": "warn"})), "tr", NOW).unwrap().level, "warn");
        assert_eq!(parse_notice(&reply(json!({"level": "red"})), "tr", NOW).unwrap().level, "info");
    }
}
