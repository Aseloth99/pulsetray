//! Latency test state: whether pings are running, what they measured, and when the last test ran.
//!
//! Pure logic (time is passed in) so it can be unit-tested without threads or a network.

use crate::ping::PingSample;
use serde::Serialize;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// History kept per target: 5 minutes at one sample per second.
pub const MAX_SAMPLES: usize = 300;

struct Run {
    /// `None` = until the user stops it.
    deadline: Option<Instant>,
    planned_secs: Option<u64>,
}

struct Target {
    name: String,
    ip: Option<String>,
    hist: VecDeque<Option<u32>>,
}

#[derive(Default)]
pub struct PingState {
    run: Option<Run>,
    /// "Continuous" mode: measure whenever the popup is open (set from the advanced view).
    pub continuous: bool,
    /// Continuous mode is waiting because a full-screen app is in front.
    pub paused: bool,
    targets: Vec<Target>,
    last_at: Option<u64>,
    last_planned: Option<u64>,
    last_reason: Option<&'static str>,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PingStatus {
    /// A timed or unlimited test is running.
    pub running: bool,
    pub unlimited: bool,
    pub remaining_secs: Option<u64>,
    pub continuous: bool,
    pub paused: bool,
    /// Unix seconds of the last finished / stopped test.
    pub last_test_at: Option<u64>,
    /// Planned length of the last test in seconds (`None` = it was unlimited).
    pub last_test_planned_secs: Option<u64>,
    /// "finished", "stopped" or "fullscreen".
    pub last_stop_reason: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetSnapshot {
    pub target: String,
    pub ip: Option<String>,
    pub hist: Vec<Option<u32>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PingSnapshot {
    pub status: PingStatus,
    pub targets: Vec<TargetSnapshot>,
}

impl PingState {
    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    /// Begin a fresh test (previous graphs are cleared). `secs = None` = unlimited.
    pub fn start(&mut self, now: Instant, secs: Option<u64>) {
        for t in &mut self.targets {
            t.hist.clear();
        }
        self.run = Some(Run {
            deadline: secs.map(|s| now + Duration::from_secs(s)),
            planned_secs: secs,
        });
        self.last_reason = None;
    }

    /// Turn continuous mode on or off. Continuous measuring replaces a running test: the test's
    /// countdown would keep ticking although measuring no longer depends on it, so it ends here.
    pub fn set_continuous(&mut self, on: bool, unix_now: u64) {
        self.continuous = on;
        if on {
            self.end("stopped", unix_now);
        }
    }

    /// Stop the running test and remember when it happened. No-op when nothing runs.
    pub fn end(&mut self, reason: &'static str, unix_now: u64) {
        if let Some(run) = self.run.take() {
            self.last_at = Some(unix_now);
            self.last_planned = run.planned_secs;
            self.last_reason = Some(reason);
        }
    }

    /// End a timed test whose time is up. Returns true when it did.
    pub fn expire(&mut self, now: Instant, unix_now: u64) -> bool {
        let due = self.run.as_ref().and_then(|r| r.deadline).is_some_and(|d| now >= d);
        if due {
            self.end("finished", unix_now);
        }
        due
    }

    /// Keep the targets in the user's order; histories of removed targets are dropped.
    pub fn sync_targets(&mut self, names: &[String]) {
        let mut next = Vec::with_capacity(names.len());
        for n in names {
            let existing = self.targets.iter().position(|t| &t.name == n);
            next.push(match existing {
                Some(i) => self.targets.swap_remove(i),
                None => Target { name: n.clone(), ip: None, hist: VecDeque::new() },
            });
        }
        self.targets = next;
    }

    pub fn record(&mut self, samples: &[PingSample]) {
        for s in samples {
            if let Some(t) = self.targets.iter_mut().find(|t| t.name == s.target) {
                t.ip = s.ip.clone();
                if t.hist.len() >= MAX_SAMPLES {
                    t.hist.pop_front();
                }
                t.hist.push_back(s.rtt);
            }
        }
    }

    pub fn status(&self, now: Instant) -> PingStatus {
        let (running, unlimited, remaining) = match &self.run {
            Some(r) => (
                true,
                r.deadline.is_none(),
                r.deadline.map(|d| d.saturating_duration_since(now).as_secs()),
            ),
            None => (false, false, None),
        };
        PingStatus {
            running,
            unlimited,
            remaining_secs: remaining,
            continuous: self.continuous,
            paused: self.paused,
            last_test_at: self.last_at,
            last_test_planned_secs: self.last_planned,
            last_stop_reason: self.last_reason.map(str::to_string),
        }
    }

    pub fn snapshot(&self, now: Instant) -> PingSnapshot {
        PingSnapshot {
            status: self.status(now),
            targets: self
                .targets
                .iter()
                .map(|t| TargetSnapshot {
                    target: t.name.clone(),
                    ip: t.ip.clone(),
                    hist: t.hist.iter().copied().collect(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: &str, rtt: Option<u32>) -> PingSample {
        PingSample { target: t.into(), ip: Some("1.2.3.4".into()), rtt }
    }

    #[test]
    fn timed_test_runs_then_expires_and_remembers_when() {
        let mut s = PingState::default();
        let t0 = Instant::now();
        s.sync_targets(&["a".into()]);
        s.start(t0, Some(60));
        assert!(s.is_running());
        assert_eq!(s.status(t0).remaining_secs, Some(60));
        assert!(!s.expire(t0 + Duration::from_secs(59), 1000));
        assert_eq!(s.status(t0 + Duration::from_secs(30)).remaining_secs, Some(30));
        assert!(s.expire(t0 + Duration::from_secs(60), 1234));
        let st = s.status(t0 + Duration::from_secs(61));
        assert!(!st.running);
        assert_eq!(st.last_test_at, Some(1234));
        assert_eq!(st.last_test_planned_secs, Some(60));
        assert_eq!(st.last_stop_reason.as_deref(), Some("finished"));
    }

    #[test]
    fn unlimited_never_expires_until_stopped() {
        let mut s = PingState::default();
        let t0 = Instant::now();
        s.start(t0, None);
        let st = s.status(t0);
        assert!(st.running && st.unlimited && st.remaining_secs.is_none());
        assert!(!s.expire(t0 + Duration::from_secs(86_400), 1));
        s.end("stopped", 77);
        let st = s.status(t0);
        assert_eq!((st.last_test_at, st.last_test_planned_secs), (Some(77), None));
        assert_eq!(st.last_stop_reason.as_deref(), Some("stopped"));
    }

    #[test]
    fn continuous_mode_replaces_a_running_test() {
        let mut s = PingState::default();
        let t0 = Instant::now();
        s.start(t0, Some(180));
        s.set_continuous(true, 42);
        let st = s.status(t0);
        assert!(!st.running && st.continuous, "the countdown must not keep running");
        assert_eq!((st.last_test_at, st.last_stop_reason.as_deref()), (Some(42), Some("stopped")));
        s.set_continuous(false, 43);
        assert!(!s.status(t0).continuous);
        assert_eq!(s.status(t0).last_test_at, Some(42), "switching it off again changes nothing else");
    }

    #[test]
    fn turning_continuous_on_while_idle_records_no_test() {
        let mut s = PingState::default();
        s.set_continuous(true, 9);
        let st = s.status(Instant::now());
        assert!(st.continuous && st.last_test_at.is_none());
    }

    #[test]
    fn ending_when_idle_changes_nothing() {
        let mut s = PingState::default();
        s.end("stopped", 5);
        assert_eq!(s.status(Instant::now()).last_test_at, None);
    }

    #[test]
    fn history_is_capped_and_cleared_by_a_new_test() {
        let mut s = PingState::default();
        s.sync_targets(&["a".into(), "b".into()]);
        for i in 0..(MAX_SAMPLES as u32 + 25) {
            s.record(&[sample("a", Some(i)), sample("b", None)]);
        }
        let snap = s.snapshot(Instant::now());
        assert_eq!(snap.targets[0].hist.len(), MAX_SAMPLES);
        assert_eq!(snap.targets[0].hist[0], Some(25), "oldest samples fall off the front");
        assert!(snap.targets[1].hist.iter().all(Option::is_none), "timeouts are kept as None");
        s.start(Instant::now(), Some(60));
        assert!(s.snapshot(Instant::now()).targets.iter().all(|t| t.hist.is_empty()));
    }

    #[test]
    fn sync_targets_keeps_order_and_drops_removed() {
        let mut s = PingState::default();
        s.sync_targets(&["a".into(), "b".into(), "c".into()]);
        s.record(&[sample("a", Some(1)), sample("b", Some(2)), sample("c", Some(3))]);
        s.sync_targets(&["c".into(), "a".into(), "new".into()]);
        let snap = s.snapshot(Instant::now());
        let names: Vec<_> = snap.targets.iter().map(|t| t.target.as_str()).collect();
        assert_eq!(names, ["c", "a", "new"]);
        assert_eq!(snap.targets[0].hist, vec![Some(3)]);
        assert_eq!(snap.targets[1].hist, vec![Some(1)]);
        assert!(snap.targets[2].hist.is_empty());
    }

    #[test]
    fn samples_for_unknown_targets_are_ignored() {
        let mut s = PingState::default();
        s.sync_targets(&["a".into()]);
        s.record(&[sample("zzz", Some(9))]);
        assert!(s.snapshot(Instant::now()).targets[0].hist.is_empty());
    }
}
