//! Idle tracking for the wipe-after-inactivity rule (CLAUDE.md, GUI rules). Time is passed in,
//! so the logic is testable without a clock.

#![allow(dead_code)] // screens use it from step 6.3

/// Seconds without input after which secret state is wiped: 5 minutes.
pub const IDLE_LIMIT_SECS: f64 = 5.0 * 60.0;

/// Remembers when the user last did something.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdleTimer {
    last_activity: Option<f64>,
}

impl IdleTimer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records activity at time `now` (seconds, any steady clock).
    pub fn touch(&mut self, now: f64) {
        self.last_activity = Some(now);
    }

    /// True when the last activity was at least [`IDLE_LIMIT_SECS`] before `now`. Before the
    /// first activity the timer starts counting at the first call.
    pub fn expired(&mut self, now: f64) -> bool {
        let last = *self.last_activity.get_or_insert(now);
        now - last >= IDLE_LIMIT_SECS
    }
}
