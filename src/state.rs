//! Failure tracking and what to say about it.
//!
//! The keepalive can only prevent *idle* death; it cannot repair a genuinely
//! broken path. So the loop's other job is to tell the user, clearly and early,
//! when replies stop coming.

/// Outcome of a single probe, as far as the state machine cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// A reply arrived.
    Reply,
    /// The probe went out but no reply came back in time.
    Timeout,
}

/// What the loop should report after a probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    /// Everything normal.
    Ok,
    /// A reply after at least one failure: the path recovered.
    Recovered { failures: u32 },
    /// A failure below the threshold; nothing to say yet.
    Quiet,
    /// The link looks down; this is reported once per outage.
    Down { failures: u32 },
    /// Still down; reported periodically so the log shows the outage is ongoing.
    StillDown { failures: u32 },
}

/// Tracks consecutive failures and decides when to speak.
#[derive(Debug, Clone)]
pub struct LinkState {
    fail_threshold: u32,
    /// Repeat a "still down" line every this many additional failures.
    repeat_every: u32,
    consecutive_failures: u32,
    down_announced: bool,
    total_probes: u64,
    total_failures: u64,
}

impl LinkState {
    pub fn new(fail_threshold: u32) -> LinkState {
        let fail_threshold = fail_threshold.max(1);
        LinkState {
            fail_threshold,
            repeat_every: fail_threshold.saturating_mul(10).max(10),
            consecutive_failures: 0,
            down_announced: false,
            total_probes: 0,
            total_failures: 0,
        }
    }

    pub fn record(&mut self, outcome: ProbeOutcome) -> Reaction {
        self.total_probes += 1;
        match outcome {
            ProbeOutcome::Reply => {
                self.consecutive_failures = 0;
                if self.down_announced {
                    self.down_announced = false;
                    Reaction::Recovered {
                        failures: self.fail_threshold,
                    }
                } else {
                    Reaction::Ok
                }
            }
            ProbeOutcome::Timeout => {
                self.total_failures += 1;
                self.consecutive_failures += 1;

                if self.consecutive_failures < self.fail_threshold {
                    return Reaction::Quiet;
                }

                if !self.down_announced {
                    self.down_announced = true;
                    return Reaction::Down {
                        failures: self.consecutive_failures,
                    };
                }

                let since_down = self.consecutive_failures - self.fail_threshold;
                if since_down > 0 && since_down % self.repeat_every == 0 {
                    Reaction::StillDown {
                        failures: self.consecutive_failures,
                    }
                } else {
                    Reaction::Quiet
                }
            }
        }
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    #[cfg(test)]
    pub fn is_down(&self) -> bool {
        self.down_announced
    }

    pub fn totals(&self) -> (u64, u64) {
        (self.total_probes, self.total_failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_from_the_start_is_quiet() {
        let mut s = LinkState::new(3);
        assert_eq!(s.record(ProbeOutcome::Reply), Reaction::Ok);
        assert_eq!(s.record(ProbeOutcome::Reply), Reaction::Ok);
        assert!(!s.is_down());
    }

    #[test]
    fn failures_below_threshold_stay_quiet() {
        let mut s = LinkState::new(3);
        assert_eq!(s.record(ProbeOutcome::Timeout), Reaction::Quiet);
        assert_eq!(s.record(ProbeOutcome::Timeout), Reaction::Quiet);
        assert!(!s.is_down());
        assert_eq!(s.consecutive_failures(), 2);
    }

    #[test]
    fn threshold_is_announced_exactly_once() {
        let mut s = LinkState::new(3);
        s.record(ProbeOutcome::Timeout);
        s.record(ProbeOutcome::Timeout);
        assert_eq!(
            s.record(ProbeOutcome::Timeout),
            Reaction::Down { failures: 3 }
        );
        assert!(s.is_down());

        for _ in 0..5 {
            assert_eq!(s.record(ProbeOutcome::Timeout), Reaction::Quiet);
        }
    }

    #[test]
    fn prolonged_outage_is_repeated_periodically() {
        let mut s = LinkState::new(2);
        s.record(ProbeOutcome::Timeout);
        assert_eq!(
            s.record(ProbeOutcome::Timeout),
            Reaction::Down { failures: 2 }
        );

        // repeat_every = max(2 * 10, 10) = 20, counted from the down point.
        let mut repeats = 0;
        for _ in 0..40 {
            if let Reaction::StillDown { .. } = s.record(ProbeOutcome::Timeout) {
                repeats += 1;
            }
        }
        assert_eq!(repeats, 2, "expected a reminder every 20 failures");
    }

    #[test]
    fn recovery_is_reported_once() {
        let mut s = LinkState::new(2);
        s.record(ProbeOutcome::Timeout);
        s.record(ProbeOutcome::Timeout);
        assert_eq!(
            s.record(ProbeOutcome::Reply),
            Reaction::Recovered { failures: 2 }
        );
        assert!(!s.is_down());
        assert_eq!(s.record(ProbeOutcome::Reply), Reaction::Ok);
    }

    #[test]
    fn a_single_blip_does_not_count_as_an_outage() {
        let mut s = LinkState::new(3);
        s.record(ProbeOutcome::Timeout);
        s.record(ProbeOutcome::Timeout);
        assert_eq!(s.record(ProbeOutcome::Reply), Reaction::Ok);
        assert!(!s.is_down());
    }

    #[test]
    fn totals_track_every_probe() {
        let mut s = LinkState::new(2);
        s.record(ProbeOutcome::Reply);
        s.record(ProbeOutcome::Timeout);
        s.record(ProbeOutcome::Reply);
        assert_eq!(s.totals(), (3, 1));
    }
}
