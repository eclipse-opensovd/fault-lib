/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 */
use alloc::collections::VecDeque;
use core::time::Duration;
use std::time::Instant;

use iceoryx2::prelude::*;
use serde::{Deserialize, Serialize};

/// IPC-safe duration with guaranteed `#[repr(C)]` layout.
///
/// `std::time::Duration` has no stable memory layout, making it unsuitable
/// for cross-process shared memory. This wrapper is used in all `#[repr(C)]`
/// types that cross IPC boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub struct IpcDuration {
    /// Whole seconds component.
    pub secs: u64,
    /// Sub-second nanoseconds component (0…999 999 999).
    pub nanos: u32,
}

impl IpcDuration {
    /// Maximum valid value for the `nanos` field.
    pub const MAX_NANOS: u32 = 999_999_999;

    /// Validates that the nanoseconds field is within the documented range (`0..=999_999_999`).
    ///
    /// Note: `Duration::new()` does not panic on out-of-range nanos - it carries
    /// excess into seconds. This method enforces the stricter invariant documented
    /// on the `nanos` field for IPC trust boundaries.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the `nanos` field exceeds `MAX_NANOS` (999,999,999).
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.nanos > Self::MAX_NANOS {
            return Err("IpcDuration nanoseconds out of range (max: 999_999_999)");
        }
        Ok(())
    }
}

impl From<Duration> for IpcDuration {
    fn from(d: Duration) -> Self {
        Self {
            secs: d.as_secs(),
            nanos: d.subsec_nanos(),
        }
    }
}

impl From<IpcDuration> for Duration {
    fn from(d: IpcDuration) -> Self {
        Duration::new(d.secs, d.nanos)
    }
}

/// Debounce descriptions capture how noisy fault sources should be filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum DebounceMode {
    /// Require N occurrences within a window to confirm fault Active.
    CountWithinWindow {
        /// Minimum number of events required within `window`.
        min_count: u32,
        /// Sliding time window for counting events.
        window: IpcDuration,
    },
    /// Confirm when signal remains bad for duration (e.g., stuck-at).
    HoldTime {
        /// Duration the fault condition must persist before confirmation.
        duration: IpcDuration,
    },
    /// Edge triggered (first occurrence) with cooldown to avoid flapping.
    EdgeWithCooldown {
        /// Minimum time between successive reports.
        cooldown: IpcDuration,
    },
}

impl DebounceMode {
    /// Create a boxed [`Debounce`] implementation matching this mode.
    #[must_use]
    pub fn into_debouncer(self) -> Box<dyn Debounce> {
        match self {
            DebounceMode::CountWithinWindow { min_count, window } => {
                Box::new(CountWithinWindow::new(min_count, window.into()))
            }
            DebounceMode::HoldTime { duration } => Box::new(HoldTime::new(duration.into())),
            DebounceMode::EdgeWithCooldown { cooldown } => {
                Box::new(EdgeWithCooldown::new(cooldown.into()))
            }
        }
    }
}

/// Debounce policy combining a mode with an optional log-throttle window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub struct DebouncePolicy {
    /// Debounce algorithm to apply.
    pub mode: DebounceMode,
    /// Optional suppression of repeats in logging within a time window.
    pub log_throttle: Option<IpcDuration>,
}

/// Trait for debounce algorithm implementations.
///
/// All implementations must be [`Send`] + [`Sync`] to support use in
/// multi-threaded fault-reporting pipelines (e.g. behind `Arc<Mutex<_>>`).
pub trait Debounce: Send + Sync {
    /// Called on each fault occurrence. Returns true if the event should be reported.
    fn on_event(&mut self, now: Instant) -> bool;

    /// Resets the internal state, e.g., after a fault clears.
    fn reset(&mut self, now: Instant);
}

/// Count-within-window debouncer.
///
/// Confirms a fault only after `min_count` events occur within a sliding
/// time window. The internal deque is capped at `min_count` entries.
pub struct CountWithinWindow {
    min_count: u32,
    window: Duration,
    occurrences: VecDeque<Instant>,
}

impl CountWithinWindow {
    /// Create a new count-within-window debouncer.
    #[must_use]
    pub fn new(min_count: u32, window: Duration) -> Self {
        Self {
            min_count,
            window,
            occurrences: VecDeque::new(),
        }
    }
}

impl Debounce for CountWithinWindow {
    fn on_event(&mut self, now: Instant) -> bool {
        while self
            .occurrences
            .front()
            .is_some_and(|&ts| now.duration_since(ts) > self.window)
        {
            self.occurrences.pop_front();
        }
        self.occurrences.push_back(now);
        // Cap the deque at min_count to prevent unbounded growth.
        // We only need the most recent min_count entries to decide.
        while self.occurrences.len() > self.min_count as usize {
            self.occurrences.pop_front();
        }
        self.occurrences.len() >= self.min_count as usize
    }

    fn reset(&mut self, _now: Instant) {
        self.occurrences.clear();
    }
}

/// Hold-time debouncer.
///
/// Confirms a fault only after the condition persists continuously for
/// the configured duration.
///
/// # Semantics
///
/// - The first call to [`on_event`](Debounce::on_event) starts an internal
///   timer but returns `false` (not yet confirmed).
/// - Subsequent calls return `true` only when the elapsed time since the
///   first event meets or exceeds `duration`.
/// - If [`reset`](Debounce::reset) is called (e.g. the fault condition
///   clears), the timer restarts from zero on the next `on_event`.
///
/// # When to use
///
/// Use `HoldTime` for "stuck-at" faults where a condition must remain
/// continuously active for a minimum period before it should be reported.
/// This prevents transient glitches from triggering fault confirmation.
///
/// # Edge cases
///
/// - A `duration` of zero confirms on the **second** call (the first
///   call always records the start time and returns `false`).
/// - Calling `reset` and then immediately `on_event` restarts the timer.
pub struct HoldTime {
    duration: Duration,
    start_time: Option<Instant>,
}

impl HoldTime {
    /// Create a new hold-time debouncer.
    #[must_use]
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            start_time: None,
        }
    }
}

impl Debounce for HoldTime {
    fn on_event(&mut self, now: Instant) -> bool {
        match self.start_time {
            None => {
                self.start_time = Some(now);
                false
            }
            Some(start) => now.duration_since(start) >= self.duration,
        }
    }

    fn reset(&mut self, _now: Instant) {
        self.start_time = None;
    }
}

/// Edge-with-cooldown debouncer.
///
/// Reports on the first occurrence, then suppresses further reports
/// until the cooldown period elapses.
pub struct EdgeWithCooldown {
    cooldown: Duration,
    last_report: Option<Instant>,
}

impl EdgeWithCooldown {
    /// Create a new edge-with-cooldown debouncer.
    #[must_use]
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            last_report: None,
        }
    }
}

impl Debounce for EdgeWithCooldown {
    fn on_event(&mut self, now: Instant) -> bool {
        match self.last_report {
            Some(last) if now.duration_since(last) < self.cooldown => false,
            _ => {
                self.last_report = Some(now);
                true
            }
        }
    }

    /// Reset the edge-with-cooldown debouncer.
    ///
    /// Sets `last_report` to `now`, which means the cooldown window
    /// restarts from this instant. Any `on_event` call arriving before
    /// `now + cooldown` will be suppressed.
    ///
    /// This is the correct behavior when the underlying fault clears:
    /// the reporter should not immediately re-fire on the next edge.
    ///
    /// # Difference from other debouncers
    ///
    /// Unlike [`HoldTime::reset`] (which clears the timer entirely),
    /// this method *anchors* a new cooldown window — the debouncer
    /// does **not** return to the "never reported" initial state.
    fn reset(&mut self, now: Instant) {
        self.last_report = Some(now);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use core::time::Duration;
    use std::time::Instant;

    use super::*;

    #[test]
    fn count_with_window_reports_only_after_min_count_within_window() {
        let now = Instant::now();
        let mut d = CountWithinWindow::new(3, Duration::from_secs(5));
        assert!(!d.on_event(now));
        assert!(!d.on_event(now + Duration::from_secs(1)));
        assert!(d.on_event(now + Duration::from_secs(2)));
        assert!(d.on_event(now + Duration::from_secs(3)));
    }

    #[test]
    fn count_with_window_drops_old_events_outside_window() {
        let mut d = CountWithinWindow::new(3, Duration::from_secs(2));
        let t0 = Instant::now();
        assert!(!d.on_event(t0));
        assert!(!d.on_event(t0 + Duration::from_secs(1)));
        assert!(d.on_event(t0 + Duration::from_secs(1)));
        assert!(!d.on_event(t0 + Duration::from_secs(4)));
        assert_eq!(d.occurrences.len(), 1);
    }

    #[test]
    fn count_with_window_reset_clears_state() {
        let mut d = CountWithinWindow::new(2, Duration::from_secs(3));
        let t0 = Instant::now();
        d.on_event(t0);
        d.on_event(t0 + Duration::from_secs(1));
        assert!(d.on_event(t0 + Duration::from_secs(2)));
        d.reset(t0 + Duration::from_secs(3));
        assert!(!d.on_event(t0 + Duration::from_secs(4)));
    }

    #[test]
    fn holdtime_requires_continuous_duration_before_report() {
        let mut d = HoldTime::new(Duration::from_secs(5));
        let t0 = Instant::now();
        assert!(!d.on_event(t0));
        assert!(!d.on_event(t0 + Duration::from_secs(3)));
        assert!(d.on_event(t0 + Duration::from_secs(6)));
    }

    #[test]
    fn holdtime_reset_resets_timer() {
        let mut d = HoldTime::new(Duration::from_secs(5));
        let t0 = Instant::now();
        d.on_event(t0);
        d.on_event(t0 + Duration::from_secs(4));
        d.reset(t0 + Duration::from_secs(5));
        assert!(!d.on_event(t0 + Duration::from_secs(6)));
    }

    #[test]
    fn edge_with_cooldown_reports_first_then_suppresses_during_cooldown() {
        let mut d = EdgeWithCooldown::new(Duration::from_secs(5));
        let t0 = Instant::now();
        assert!(d.on_event(t0));
        assert!(!d.on_event(t0 + Duration::from_secs(2)));
        assert!(d.on_event(t0 + Duration::from_secs(6)));
    }

    #[test]
    fn edge_with_cooldown_reset_forces_new_last_report() {
        let mut d = EdgeWithCooldown::new(Duration::from_secs(5));
        let t0 = Instant::now();
        d.on_event(t0);
        d.reset(t0 + Duration::from_secs(2));
        assert!(!d.on_event(t0 + Duration::from_secs(4)));
        assert!(d.on_event(t0 + Duration::from_secs(8)));
    }

    #[test]
    fn debounce_mode_creates_proper_implementations() {
        let d1 = DebounceMode::CountWithinWindow {
            min_count: 2,
            window: Duration::from_secs(3).into(),
        }
        .into_debouncer();
        let d2 = DebounceMode::HoldTime {
            duration: Duration::from_secs(1).into(),
        }
        .into_debouncer();
        let d3 = DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_secs(10).into(),
        }
        .into_debouncer();

        let now = Instant::now();
        for mut d in [d1, d2, d3] {
            d.on_event(now);
            d.reset(now);
        }
    }

    #[test]
    fn debounce_policy_derive_traits_work() {
        let p1 = DebouncePolicy {
            mode: DebounceMode::HoldTime {
                duration: Duration::from_secs(2).into(),
            },
            log_throttle: Some(Duration::from_secs(10).into()),
        };
        let p2 = p1.clone();
        assert_eq!(p1, p2);
        assert!(format!("{p1:?}").contains("HoldTime"));
    }

    #[test]
    fn ipc_duration_roundtrip() {
        let original = Duration::new(42, 999_999_999);
        let ipc: IpcDuration = original.into();
        let back: Duration = ipc.into();
        assert_eq!(original, back);
    }

    #[test]
    fn ipc_duration_zero() {
        let ipc: IpcDuration = Duration::ZERO.into();
        assert_eq!(ipc.secs, 0);
        assert_eq!(ipc.nanos, 0);
        let back: Duration = ipc.into();
        assert_eq!(back, Duration::ZERO);
    }

    #[test]
    fn ipc_duration_max_nanos() {
        let original = Duration::new(u64::MAX, 999_999_999);
        let ipc: IpcDuration = original.into();
        assert_eq!(ipc.secs, u64::MAX);
        assert_eq!(ipc.nanos, 999_999_999);
    }

    #[test]
    fn ipc_duration_size_and_alignment() {
        assert_eq!(core::mem::size_of::<IpcDuration>(), 16);
        assert_eq!(core::mem::align_of::<IpcDuration>(), 8);
    }

    #[test]
    fn count_with_window_deque_capped_at_min_count() {
        let mut d = CountWithinWindow::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        // Push 10 events — all within the window
        for i in 0..10u32 {
            d.on_event(t0 + Duration::from_millis(u64::from(i)));
        }
        // Deque should never grow beyond min_count (3)
        assert!(
            d.occurrences.len() <= 3,
            "Deque should be capped at min_count, got: {}",
            d.occurrences.len()
        );
    }

    #[test]
    fn count_with_window_cap_preserves_correctness() {
        let mut d = CountWithinWindow::new(3, Duration::from_secs(5));
        let t0 = Instant::now();
        // First 2 events: not enough
        assert!(!d.on_event(t0));
        assert!(!d.on_event(t0 + Duration::from_secs(1)));
        // Third event: min_count reached
        assert!(d.on_event(t0 + Duration::from_secs(2)));
        // Many more events, deque stays bounded
        for i in 3..100u32 {
            assert!(d.on_event(t0 + Duration::from_millis(2000 + u64::from(i))));
        }
        assert!(d.occurrences.len() <= 3);
    }
}
