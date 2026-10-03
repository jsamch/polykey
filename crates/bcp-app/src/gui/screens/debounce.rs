//! Debouncing for live previews: a burst of changes starts one job, after the changes stop.
//! Time is passed in, so the logic is testable without a clock.

/// Tracks the latest change and decides when a job may start. Every change bumps a generation
/// counter; a job is tagged with the generation it was started for, and a result whose
/// generation is no longer current is stale and must be discarded.
#[derive(Clone, Copy, Debug)]
pub struct Debouncer {
    delay: f64,
    generation: u64,
    /// When the pending job becomes due, `None` when nothing waits.
    due: Option<f64>,
}

impl Debouncer {
    /// A debouncer that waits `delay` seconds after the last change.
    pub fn new(delay: f64) -> Self {
        Debouncer {
            delay,
            generation: 0,
            due: None,
        }
    }

    /// Records a change at time `now`. Returns the new generation.
    pub fn changed(&mut self, now: f64) -> u64 {
        self.generation += 1;
        self.due = Some(now + self.delay);
        self.generation
    }

    /// Drops the pending job and makes every earlier generation stale (the settings are no
    /// longer previewable).
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.due = None;
    }

    /// Returns the generation to start a job for, once, when the delay since the last change
    /// has passed and a job may start. While `can_start` is false the job stays pending.
    pub fn poll(&mut self, now: f64, can_start: bool) -> Option<u64> {
        match self.due {
            Some(due) if now >= due && can_start => {
                self.due = None;
                Some(self.generation)
            }
            _ => None,
        }
    }

    /// True when a result tagged `generation` still matches the latest change.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// Seconds until the pending job is due, `None` when nothing waits.
    pub fn remaining(&self, now: f64) -> Option<f64> {
        self.due.map(|d| (d - now).max(0.0))
    }
}
