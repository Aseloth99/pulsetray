//! Where an update stands: which version is on offer, what is happening to it (nothing / downloading / downloaded /
//! installing / failed) and whether anybody wants it installed right now.
//!
//! The daily check, the background download of the automatic mode, the popup's update screen and the tray menu all
//! look at (and press buttons on) this one machine, so there is exactly one download at a time: an automatic download
//! that is running is joined by the user's "Update" press instead of being started a second time. No windows, no
//! network and no clock in here, which is what makes the rules testable (see the tests); lib.rs runs the worker
//! thread that asks it what to do next.

use crate::updater::{Downloaded, Offer, UpdErr};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Serialised as `{"state": "failed", "error": "..."}` (`error` only for a failure).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(tag = "state", content = "error", rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Downloading,
    /// Downloaded and verified, waiting for a quiet moment (automatic mode) or for the install that is about to start.
    Ready,
    Installing,
    Failed(String),
}

/// What `start` did.
#[derive(Debug, PartialEq)]
pub enum Start {
    /// A new job was begun: the caller must run `lib.rs::run_update_job` on a thread.
    Spawn,
    /// A job is already running (and, for the user's press, will install as soon as it has the package).
    Joined,
    /// Already installing.
    Installing,
    NoOffer,
}

/// What the worker should do next.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Nothing to install (the offer is gone): end the job.
    Stop,
    /// No package for the offered version yet.
    Download,
    /// Downloaded, but nobody wants it right now.
    Wait,
    Install,
}

/// What became of a finished download.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Ready,
    /// The offer changed while it was coming in: that file is no use, ask again what to do.
    Stale,
    /// The user stopped it; the job is over.
    Cancelled,
    /// It failed; the job is over (the phase says why).
    Failed,
}

#[derive(Default)]
struct Inner {
    offer: Option<Offer>,
    phase: Phase,
    pkg: Option<Downloaded>,
    /// A worker thread is alive (downloading, waiting or installing).
    job: bool,
    /// The job was begun by the automatic mode (it waits for a quiet moment unless `install_now`).
    auto: bool,
    /// The user pressed Update: install as soon as the package is verified, popup open or not.
    install_now: bool,
    /// The user stopped the download, but the worker has not noticed yet (it may sit in a stalled read).
    cancelling: bool,
}

pub struct UpdateState {
    inner: Mutex<Inner>,
    /// Read by the download between two chunks.
    cancel: Arc<AtomicBool>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self::new()
    }
}

fn discard(pkg: Option<Downloaded>) {
    if let Some(p) = pkg {
        let _ = std::fs::remove_file(p.path);
    }
}

impl UpdateState {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner::default()), cancel: Arc::new(AtomicBool::new(false)) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn offer(&self) -> Option<Offer> {
        self.lock().offer.clone()
    }

    pub fn phase(&self) -> Phase {
        self.lock().phase.clone()
    }

    /// The flag the download polls.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// A server answer. Returns whether the offer itself changed (the UI is told then).
    pub fn set_offer(&self, offer: Option<Offer>) -> bool {
        let mut g = self.lock();
        let changed = g.offer != offer;
        let version = |o: &Option<Offer>| o.as_ref().map(|o| o.version.clone());
        let new_version = version(&offer);
        // A package of another version than the one on offer is of no use any more.
        if g.pkg.as_ref().map(|p| Some(p.version.clone())).is_some_and(|v| v != new_version) && g.phase != Phase::Installing {
            discard(g.pkg.take());
        }
        // A result (error / "downloaded") that belonged to another version is stale.
        if !g.job && matches!(g.phase, Phase::Failed(_) | Phase::Ready) && version(&g.offer) != new_version {
            g.phase = Phase::Idle;
        }
        g.offer = offer;
        changed
    }

    /// "Update" pressed (`manual`) or the automatic mode wants the update. The caller starts the worker on `Spawn`.
    pub fn start(&self, manual: bool) -> Start {
        let mut g = self.lock();
        if g.offer.is_none() {
            return Start::NoOffer;
        }
        if g.phase == Phase::Installing {
            return Start::Installing;
        }
        if g.cancelling {
            // The user stopped the download a moment ago and the worker is still winding down. The automatic mode
            // respects that; a new press of the button takes the stop back: the same download simply goes on.
            if !manual {
                return Start::Joined;
            }
            g.cancelling = false;
            g.install_now = true;
            g.phase = Phase::Downloading;
            self.cancel.store(false, Ordering::SeqCst);
            return Start::Joined;
        }
        if manual {
            g.install_now = true;
        }
        if g.job {
            return Start::Joined;
        }
        g.job = true;
        g.auto = !manual;
        self.cancel.store(false, Ordering::SeqCst);
        let have = g.pkg.as_ref().map(|p| &p.version) == g.offer.as_ref().map(|o| &o.version);
        g.phase = if have { Phase::Ready } else { Phase::Downloading };
        Start::Spawn
    }

    /// The worker asks what to do. `quiet` = a good moment for an automatic install (popup closed, no test running).
    pub fn next_step(&self, quiet: bool) -> Step {
        let g = self.lock();
        match (&g.offer, &g.pkg) {
            (None, _) => Step::Stop,
            (Some(o), Some(p)) if p.version == o.version => {
                if g.install_now || (g.auto && quiet) {
                    Step::Install
                } else {
                    Step::Wait
                }
            }
            (Some(_), _) => Step::Download,
        }
    }

    /// The worker is about to download; what it should fetch (None: the offer is gone).
    pub fn begin_download(&self) -> Option<Offer> {
        let mut g = self.lock();
        let offer = g.offer.clone()?;
        discard(g.pkg.take());
        if !g.cancelling {
            g.phase = Phase::Downloading;
        }
        Some(offer)
    }

    /// The worker's download is over.
    pub fn finish_download(&self, result: Result<Downloaded, UpdErr>) -> Outcome {
        let mut g = self.lock();
        let stopped = |g: &mut Inner, cancel: &AtomicBool| {
            g.cancelling = false;
            g.job = false;
            g.install_now = false;
            g.phase = Phase::Idle;
            cancel.store(false, Ordering::SeqCst);
        };
        match result {
            Err(UpdErr::Cancelled) if g.cancelling => {
                stopped(&mut *g, &self.cancel);
                Outcome::Cancelled
            }
            // The stop was taken back (see `start`) just as the worker noticed it: carry on from the top.
            Err(UpdErr::Cancelled) => Outcome::Stale,
            // Finished (or failed) at the very moment the user stopped it: the stop wins.
            r if g.cancelling => {
                discard(r.ok());
                stopped(&mut *g, &self.cancel);
                Outcome::Cancelled
            }
            Err(e) => {
                g.phase = Phase::Failed(e.ui());
                g.job = false;
                g.install_now = false;
                Outcome::Failed
            }
            Ok(p) => {
                if g.offer.as_ref().map(|o| &o.version) != Some(&p.version) {
                    discard(Some(p));
                    return Outcome::Stale;
                }
                g.pkg = Some(p);
                g.phase = Phase::Ready;
                Outcome::Ready
            }
        }
    }

    /// The worker is about to install: the package to install (None: it is gone / not the offered version).
    pub fn begin_install(&self) -> Option<Downloaded> {
        let mut g = self.lock();
        let p = g.pkg.clone()?;
        if g.offer.as_ref().map(|o| &o.version) != Some(&p.version) {
            return None;
        }
        g.phase = Phase::Installing;
        Some(p)
    }

    /// Installing failed: the old version keeps running, the error is shown, "Retry" starts from scratch.
    pub fn install_failed(&self, error: String) {
        let mut g = self.lock();
        discard(g.pkg.take());
        g.phase = Phase::Failed(error);
        g.job = false;
        g.install_now = false;
    }

    /// The offer is gone while a job was waiting: nothing to do any more.
    pub fn end_job(&self) {
        let mut g = self.lock();
        g.job = false;
        g.install_now = false;
        g.cancelling = false;
        if !matches!(g.phase, Phase::Failed(_)) {
            g.phase = Phase::Idle;
        }
    }

    /// "Cancel" on the update screen: only a download can be stopped. The screen goes back to "idle" at once; the
    /// worker follows when its read returns (a stalled connection may take a while).
    pub fn cancel(&self) -> bool {
        let mut g = self.lock();
        if g.phase != Phase::Downloading {
            return false;
        }
        g.cancelling = true;
        g.install_now = false;
        g.phase = Phase::Idle;
        self.cancel.store(true, Ordering::SeqCst);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn offer(version: &str) -> Offer {
        Offer {
            version: version.into(),
            notes: String::new(),
            size: Some(1000),
            url: "https://macik.com.tr/x".into(),
            sha256: Some("a".repeat(64)),
            mandatory: false,
        }
    }

    /// A downloaded package whose file really exists (so that "dropped" can be seen).
    fn pkg(version: &str, name: &str) -> Downloaded {
        let path: PathBuf = std::env::temp_dir().join(format!("pt_state_{}_{name}.pkg", std::process::id()));
        std::fs::write(&path, b"MZ").unwrap();
        Downloaded { version: version.into(), path, sha256: "b".repeat(64) }
    }

    fn with_offer(v: &str) -> UpdateState {
        let s = UpdateState::new();
        s.set_offer(Some(offer(v)));
        s
    }

    #[test]
    fn nothing_to_start_without_an_offer() {
        let s = UpdateState::new();
        assert_eq!(s.start(true), Start::NoOffer);
        assert_eq!(s.start(false), Start::NoOffer);
        assert_eq!(s.phase(), Phase::Idle);
        assert_eq!(s.next_step(true), Step::Stop);
        assert!(!s.cancel());
    }

    #[test]
    fn only_one_download_at_a_time_and_the_users_press_joins_an_automatic_one() {
        let s = with_offer("1.1.0");
        // the automatic mode begins: a worker is needed
        assert_eq!(s.start(false), Start::Spawn);
        assert_eq!(s.phase(), Phase::Downloading);
        assert_eq!(s.next_step(true), Step::Download);
        // a second automatic start (the next check) and the user's press both join it: no second worker
        assert_eq!(s.start(false), Start::Joined);
        assert_eq!(s.start(true), Start::Joined);
        assert_eq!(s.start(true), Start::Joined, "pressing twice is harmless");
        // the download comes in: because the user pressed, it is installed at once, popup open or not
        assert_eq!(s.begin_download().unwrap().version, "1.1.0");
        assert_eq!(s.finish_download(Ok(pkg("1.1.0", "join"))), Outcome::Ready);
        assert_eq!(s.phase(), Phase::Ready);
        assert_eq!(s.next_step(false), Step::Install);
        assert_eq!(s.begin_install().unwrap().version, "1.1.0");
        assert_eq!(s.phase(), Phase::Installing);
        assert_eq!(s.start(true), Start::Installing, "no second install");
        assert_eq!(s.start(false), Start::Installing);
        discard(s.begin_install());
    }

    #[test]
    fn the_automatic_mode_waits_for_a_quiet_moment_the_button_does_not() {
        let s = with_offer("1.1.0");
        assert_eq!(s.start(false), Start::Spawn);
        s.begin_download();
        assert_eq!(s.finish_download(Ok(pkg("1.1.0", "quiet"))), Outcome::Ready);
        assert_eq!(s.next_step(false), Step::Wait, "popup open: not now");
        assert_eq!(s.next_step(false), Step::Wait);
        assert_eq!(s.next_step(true), Step::Install, "popup closed");
        // the user presses Update while it is waiting: the running worker is told, no new one starts
        assert_eq!(s.start(true), Start::Joined);
        assert_eq!(s.next_step(false), Step::Install);
        discard(s.begin_install());
    }

    #[test]
    fn a_manual_start_downloads_and_installs_without_waiting() {
        let s = with_offer("1.1.0");
        assert_eq!(s.start(true), Start::Spawn);
        s.begin_download();
        s.finish_download(Ok(pkg("1.1.0", "manual")));
        assert_eq!(s.next_step(false), Step::Install);
        discard(s.begin_install());
    }

    #[test]
    fn cancel_stops_a_download_and_the_screen_is_idle_at_once() {
        let s = with_offer("1.1.0");
        assert_eq!(s.start(true), Start::Spawn);
        s.begin_download();
        let flag = s.cancel_flag();
        assert!(!flag.load(Ordering::SeqCst));
        assert!(s.cancel());
        assert!(flag.load(Ordering::SeqCst), "the download is told");
        assert_eq!(s.phase(), Phase::Idle, "the UI does not wait for the worker");
        assert!(!s.cancel(), "nothing left to cancel");
        // the worker's read returns with the stop
        assert_eq!(s.finish_download(Err(UpdErr::Cancelled)), Outcome::Cancelled);
        assert_eq!(s.phase(), Phase::Idle);
        assert!(s.offer().is_some(), "the offer stays: the pill and the menu entry remain");
        // and a new press starts afresh
        assert_eq!(s.start(true), Start::Spawn);
        assert_eq!(s.phase(), Phase::Downloading);
        assert!(!flag.load(Ordering::SeqCst), "the flag was reset");
    }

    #[test]
    fn pressing_update_again_while_the_cancelled_download_winds_down_just_goes_on() {
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        assert!(s.cancel());
        // the worker is stuck in a stalled read; the user changes their mind
        assert_eq!(s.start(true), Start::Joined, "no second worker next to the old one");
        assert_eq!(s.phase(), Phase::Downloading);
        assert!(!s.cancel_flag().load(Ordering::SeqCst));
        // the old worker's read comes back: with the flag cleared, it goes on to install
        let p = pkg("1.1.0", "again");
        assert_eq!(s.finish_download(Ok(p)), Outcome::Ready);
        assert_eq!(s.next_step(false), Step::Install);
        discard(s.begin_install());
    }

    #[test]
    fn the_automatic_mode_does_not_undo_a_cancel() {
        let s = with_offer("1.1.0");
        s.start(false);
        s.begin_download();
        assert!(s.cancel());
        assert_eq!(s.start(false), Start::Joined, "the next check must not restart what the user stopped");
        assert_eq!(s.phase(), Phase::Idle);
        assert_eq!(s.finish_download(Err(UpdErr::Cancelled)), Outcome::Cancelled);
        assert_eq!(s.phase(), Phase::Idle);
    }

    #[test]
    fn a_download_that_finishes_just_as_the_user_cancels_is_thrown_away() {
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        assert!(s.cancel());
        let p = pkg("1.1.0", "race");
        let path = p.path.clone();
        assert_eq!(s.finish_download(Ok(p)), Outcome::Cancelled);
        assert!(!path.exists(), "no half-wanted package stays in %TEMP%");
        assert_eq!(s.phase(), Phase::Idle);
        assert_eq!(s.next_step(true), Step::Download);
    }

    #[test]
    fn a_failure_shows_up_and_retry_starts_again() {
        let s = with_offer("1.1.0");
        assert_eq!(s.start(true), Start::Spawn);
        s.begin_download();
        assert_eq!(s.finish_download(Err(UpdErr::Network("timed out".into()))), Outcome::Failed);
        assert_eq!(s.phase(), Phase::Failed("network: timed out".into()));
        assert!(s.offer().is_some(), "the offer stays so that the user can try again");
        assert!(!s.cancel(), "nothing to cancel in a failed state");
        assert_eq!(s.start(true), Start::Spawn, "Retry");
        assert_eq!(s.phase(), Phase::Downloading);
        s.begin_download();
        assert_eq!(s.finish_download(Err(UpdErr::Checksum)), Outcome::Failed);
        assert_eq!(s.phase(), Phase::Failed("checksum".into()));
        // the automatic mode tries again at the next check too
        assert_eq!(s.start(false), Start::Spawn);
    }

    #[test]
    fn a_failed_install_drops_the_package_and_keeps_the_offer() {
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        let p = pkg("1.1.0", "instfail");
        let path = p.path.clone();
        s.finish_download(Ok(p));
        assert!(s.begin_install().is_some());
        s.install_failed("not_writable: denied".into());
        assert_eq!(s.phase(), Phase::Failed("not_writable: denied".into()));
        assert!(!path.exists(), "the next try downloads afresh (and re-verifies)");
        assert!(s.offer().is_some());
        assert_eq!(s.start(true), Start::Spawn);
    }

    #[test]
    fn a_newer_offer_replaces_a_waiting_package() {
        let s = with_offer("1.1.0");
        s.start(false);
        s.begin_download();
        let old = pkg("1.1.0", "stale1");
        let old_path = old.path.clone();
        s.finish_download(Ok(old));
        assert_eq!(s.next_step(false), Step::Wait);
        // the server now offers 1.2.0 (the worker is still waiting)
        assert!(s.set_offer(Some(offer("1.2.0"))));
        assert!(!old_path.exists(), "the 1.1.0 package is dropped");
        assert_eq!(s.next_step(true), Step::Download, "the worker fetches the new one");
        assert_eq!(s.begin_download().unwrap().version, "1.2.0");
        let new = pkg("1.2.0", "stale2");
        assert_eq!(s.finish_download(Ok(new)), Outcome::Ready);
        assert_eq!(s.next_step(true), Step::Install);
        assert_eq!(s.begin_install().unwrap().version, "1.2.0");
        discard(s.begin_install());
    }

    #[test]
    fn a_download_that_arrives_after_the_offer_changed_is_not_used() {
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        s.set_offer(Some(offer("1.2.0")));
        let late = pkg("1.1.0", "late");
        let path = late.path.clone();
        assert_eq!(s.finish_download(Ok(late)), Outcome::Stale);
        assert!(!path.exists());
        assert_eq!(s.next_step(false), Step::Download);
        // and an install can never take a package that is not the offered version
        assert!(s.begin_install().is_none());
    }

    #[test]
    fn a_withdrawn_offer_ends_the_job_and_the_old_error_goes() {
        let s = with_offer("1.1.0");
        s.start(false);
        s.begin_download();
        let p = pkg("1.1.0", "gone");
        let path = p.path.clone();
        s.finish_download(Ok(p));
        assert!(s.set_offer(None));
        assert!(!path.exists());
        assert_eq!(s.next_step(true), Step::Stop);
        s.end_job();
        assert_eq!(s.phase(), Phase::Idle);
        assert_eq!(s.start(true), Start::NoOffer);

        // an error that belonged to 1.1.0 is not shown for 1.2.0
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        s.finish_download(Err(UpdErr::Http(500)));
        assert_eq!(s.phase(), Phase::Failed("http: 500".into()));
        assert!(!s.set_offer(Some(offer("1.1.0"))), "same answer again: nothing changed");
        assert_eq!(s.phase(), Phase::Failed("http: 500".into()));
        assert!(s.set_offer(Some(offer("1.2.0"))));
        assert_eq!(s.phase(), Phase::Idle);
    }

    #[test]
    fn refreshing_the_same_offer_leaves_a_running_download_alone() {
        let s = with_offer("1.1.0");
        s.start(true);
        s.begin_download();
        let mut again = offer("1.1.0");
        again.notes = "edited notes".into();
        assert!(s.set_offer(Some(again)), "the notes changed, the UI is told");
        assert_eq!(s.phase(), Phase::Downloading);
        assert_eq!(s.start(true), Start::Joined);
    }

    #[test]
    fn the_phase_reaches_the_ui_as_state_and_error() {
        let j = |p: Phase| serde_json::to_value(p).unwrap();
        assert_eq!(j(Phase::Idle), serde_json::json!({"state": "idle"}));
        assert_eq!(j(Phase::Downloading), serde_json::json!({"state": "downloading"}));
        assert_eq!(j(Phase::Ready), serde_json::json!({"state": "ready"}));
        assert_eq!(j(Phase::Installing), serde_json::json!({"state": "installing"}));
        assert_eq!(j(Phase::Failed("checksum".into())), serde_json::json!({"state": "failed", "error": "checksum"}));
    }
}
