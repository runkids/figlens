//! When a watch runner refreshes one source (#92 B2): pure state, no I/O. The runner polls the
//! source's stamp, feeds each result to `Detector::observe`, and submits a content-verified
//! refresh when told; the job stages, hashes and compares the bytes, so this never reads them.
// shortcut: no runner calls it yet; B2's runner lands with B1's tracking tables.
#![cfg_attr(not(test), allow(dead_code))]

use std::time::{Duration, Instant};

/// How often the runner stats a watched source.
pub(crate) const POLL: Duration = Duration::from_millis(250);
/// A changed stamp must hold this long before its save counts as finished.
const STABLE: Duration = Duration::from_millis(300);
/// Continuous churn refreshes anyway after this; the job's staged read retries a moving file.
const CHURN: Duration = Duration::from_millis(1200);
/// A save can keep mtime and length (or replace the inode with equal ones): the content is
/// verified this often, so such a change is seen within this bound plus the refresh.
pub(crate) const VERIFY: Duration = Duration::from_secs(60);
/// A missing source (mid replace-save) is reported only once it stays missing this long.
const MISSING: Duration = Duration::from_secs(2);
/// A busy (locked, access-denied) source is `SourceBusy` after this, `SourceUnreadable` after
/// `UNREADABLE`; it is retried with a backoff doubling from `POLL` up to `BACKOFF`.
const BUSY: Duration = Duration::from_secs(2);
const UNREADABLE: Duration = Duration::from_secs(60);
const BACKOFF: Duration = Duration::from_secs(5);

/// One stat of the source.
pub(crate) enum Probe<S> {
    Stamp(S),
    Missing,
    /// Locked or access denied: transient until `UNREADABLE`.
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Watching,
    SourceBusy,
    SourceMissing,
    SourceUnreadable,
}

/// What the runner does after one observation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// Submit one content-verified refresh now.
    pub submit: bool,
    pub state: State,
    /// When to probe again.
    pub next: Instant,
}

pub(crate) struct Detector<S> {
    /// The stamp the last submitted refresh started from; `None` forces one, so a runner that
    /// (re)starts verifies saves made while nobody watched.
    seen: Option<S>,
    /// A newer stamp, since when it held, and since when stamps kept changing.
    pending: Option<(S, Instant, Instant)>,
    verify_at: Instant,
    missing: Option<Instant>,
    busy: Option<(Instant, Duration)>,
    running: bool,
    dirty: bool,
}

impl<S: Clone + PartialEq> Detector<S> {
    pub(crate) fn new(now: Instant) -> Self {
        Detector {
            seen: None,
            pending: None,
            verify_at: now,
            missing: None,
            busy: None,
            running: false,
            dirty: false,
        }
    }

    pub(crate) fn observe(&mut self, now: Instant, probe: Probe<S>) -> Step {
        let stamp = match probe {
            Probe::Stamp(stamp) => stamp,
            Probe::Missing => {
                self.busy = None;
                let since = *self.missing.get_or_insert(now);
                let state = if now - since >= MISSING {
                    State::SourceMissing
                } else {
                    State::Watching
                };
                return self.wait(now + POLL, state);
            }
            Probe::Busy => {
                self.missing = None;
                let (since, delay) = match self.busy {
                    Some((since, delay)) => (since, (delay * 2).min(BACKOFF)),
                    None => (now, POLL),
                };
                self.busy = Some((since, delay));
                let state = match now - since {
                    held if held >= UNREADABLE => State::SourceUnreadable,
                    held if held >= BUSY => State::SourceBusy,
                    _ => State::Watching,
                };
                return self.wait(now + delay, state);
            }
        };
        self.missing = None;
        self.busy = None;
        if self.seen.as_ref() != Some(&stamp) {
            match &mut self.pending {
                Some((held, since, _)) if *held != stamp => {
                    *held = stamp.clone();
                    *since = now;
                }
                Some(_) => {}
                None => self.pending = Some((stamp.clone(), now, now)),
            }
        } else {
            self.pending = None;
        }
        let settled = self
            .pending
            .as_ref()
            .is_some_and(|(_, since, churn)| now - *since >= STABLE || now - *churn >= CHURN);
        if settled || (self.pending.is_none() && now >= self.verify_at) {
            self.pending = None;
            self.seen = Some(stamp);
            return self.request(now);
        }
        self.wait(now + POLL, State::Watching)
    }

    /// The submitted refresh ended; `failed` (the file kept changing, or any job error) makes
    /// the next stable stamp refresh again. Returns the follow-up for a change seen meanwhile.
    pub(crate) fn finished(&mut self, now: Instant, failed: bool) -> Step {
        self.running = false;
        if failed {
            self.seen = None;
        }
        if std::mem::take(&mut self.dirty) {
            return self.request(now);
        }
        self.wait(now + POLL, State::Watching)
    }

    fn request(&mut self, now: Instant) -> Step {
        self.verify_at = now + VERIFY;
        if self.running {
            self.dirty = true;
            return self.wait(now + POLL, State::Watching);
        }
        self.running = true;
        Step {
            submit: true,
            state: State::Watching,
            next: now + POLL,
        }
    }

    fn wait(&self, next: Instant, state: State) -> Step {
        Step {
            submit: false,
            state,
            next,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `probes` at `POLL` steps from `t0`; returns the times (ms after `t0`) it submitted.
    fn run(d: &mut Detector<u32>, t0: Instant, probes: &[(u64, Option<u32>)]) -> Vec<u64> {
        let mut submits = Vec::new();
        for &(ms, stamp) in probes {
            let probe = stamp.map_or(Probe::Missing, Probe::Stamp);
            if d.observe(t0 + Duration::from_millis(ms), probe).submit {
                submits.push(ms);
                d.finished(t0 + Duration::from_millis(ms), false);
            }
        }
        submits
    }

    fn ticks(from: u64, to: u64, stamp: Option<u32>) -> Vec<(u64, Option<u32>)> {
        (from..to).step_by(250).map(|ms| (ms, stamp)).collect()
    }

    #[test]
    fn a_new_detector_verifies_the_source_once_its_stamp_holds() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        assert_eq!(run(&mut d, t0, &ticks(0, 3000, Some(1))), [500]);
    }

    #[test]
    fn a_save_refreshes_once_after_its_stamp_holds() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        assert_eq!(run(&mut d, t0, &ticks(1000, 4000, Some(2))), [1500]);
    }

    #[test]
    fn churn_refreshes_at_the_cap_instead_of_waiting_forever() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        let churn: Vec<_> = (0..8)
            .map(|i| (1000 + i * 250, Some(10 + i as u32)))
            .collect();
        assert_eq!(run(&mut d, t0, &churn), [2250]);
    }

    #[test]
    fn an_unchanged_stamp_is_content_verified_every_interval() {
        let t0 = Instant::now();
        let mut d = Detector::new(t0);
        let every = VERIFY.as_millis() as u64;
        assert_eq!(
            run(&mut d, t0, &ticks(0, 2 * every + 750, Some(1))),
            [500, 500 + every, 500 + 2 * every]
        );
    }

    #[test]
    fn a_change_during_a_refresh_gives_exactly_one_follow_up() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        assert!(!d.observe(at(0), Probe::Stamp(1)).submit);
        assert!(d.observe(at(300), Probe::Stamp(1)).submit);
        for (ms, stamp) in [(600, 2), (1000, 2), (1300, 3), (1700, 3)] {
            assert!(!d.observe(at(ms), Probe::Stamp(stamp)).submit);
        }
        assert!(d.finished(at(1800), false).submit);
        assert!(!d.finished(at(2000), false).submit);
    }

    #[test]
    fn a_failed_refresh_is_retried_on_the_next_stable_stamp() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        d.observe(at(0), Probe::Stamp(1));
        assert!(d.observe(at(300), Probe::Stamp(1)).submit);
        d.finished(at(400), true);
        assert!(!d.observe(at(500), Probe::Stamp(1)).submit);
        assert!(d.observe(at(800), Probe::Stamp(1)).submit);
    }

    #[test]
    fn a_missing_source_is_silent_while_brief_then_reported_without_a_refresh() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Detector::new(t0);
        run(&mut d, t0, &ticks(0, 1000, Some(1)));
        let brief = d.observe(at(1000), Probe::Missing);
        assert_eq!((brief.submit, brief.state), (false, State::Watching));
        let long = d.observe(at(3000), Probe::Missing);
        assert_eq!((long.submit, long.state), (false, State::SourceMissing));
        // A replace-save that comes back with a new stamp refreshes once it holds.
        assert_eq!(run(&mut d, t0, &ticks(3250, 4500, Some(2))), [3750]);
    }

    #[test]
    fn a_busy_source_backs_off_to_the_cap_and_never_refreshes() {
        let t0 = Instant::now();
        let mut d: Detector<u32> = Detector::new(t0);
        let mut now = t0;
        let mut seen = Vec::new();
        while now - t0 < UNREADABLE + BACKOFF {
            let step = d.observe(now, Probe::Busy);
            assert!(!step.submit);
            seen.push((step.next - now, step.state));
            now = step.next;
        }
        let delays: Vec<_> = seen.iter().map(|(d, _)| d.as_millis()).take(6).collect();
        assert_eq!(delays, [250, 500, 1000, 2000, 4000, 5000]);
        assert!(seen.iter().all(|(d, _)| *d <= BACKOFF));
        assert_eq!(seen.first().unwrap().1, State::Watching);
        assert!(seen.iter().any(|(_, s)| *s == State::SourceBusy));
        assert_eq!(seen.last().unwrap().1, State::SourceUnreadable);
    }
}
