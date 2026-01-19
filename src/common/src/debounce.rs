// Copyright (c) 2025 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// This program and the accompanying materials are made available under the
// terms of the Apache License Version 2.0 which is available at
// <https://www.apache.org/licenses/LICENSE-2.0>
//
// SPDX-License-Identifier: Apache-2.0
//
use iceoryx2::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

// Debounce descriptions capture how noisy fault sources should be filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum DebounceMode {
    /// Require N occurrences within a window to confirm fault Active.
    CountWithinWindow { min_count: u32, window: Duration },
    /// Confirm when signal remains bad for duration (e.g., stuck-at).
    HoldTime { duration: Duration },
    /// Edge triggered (first occurrence) with cooldown to avoid flapping.
    EdgeWithCooldown { cooldown: Duration },
}

impl DebounceMode {
    pub fn into(self) -> Box<dyn Debounce + Send> {
        match self {
            DebounceMode::CountWithinWindow { min_count, window } => Box::new(CountWithinWindow::new(min_count, window)),
            DebounceMode::HoldTime { duration } => Box::new(HoldTime::new(duration)),
            DebounceMode::EdgeWithCooldown { cooldown } => Box::new(EdgeWithCooldown::new(cooldown)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub struct DebouncePolicy {
    pub mode: DebounceMode,
    /// Optional suppression of repeats in logging within a time window.
    pub log_throttle: Option<Duration>,
}

pub trait Debounce {
    /// Called on each fault occurrence. Returns true if the event should be reported.
    fn on_event(&mut self, now: Instant) -> bool;

    /// Resets the internal state, e.g., after a fault clears.
    fn reset(&mut self, now: Instant);
}

pub struct CountWithinWindow {
    min_count: u32,
    window: Duration,
    occurrences: VecDeque<Instant>,
}

impl CountWithinWindow {
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
        while self.occurrences.front().is_some_and(|&ts| now.duration_since(ts) > self.window) {
            self.occurrences.pop_front();
        }
        self.occurrences.push_back(now);
        (self.occurrences.len() as u32) >= self.min_count
    }

    fn reset(&mut self, _now: Instant) {
        self.occurrences.clear();
    }
}

pub struct HoldTime {
    duration: Duration,
    start_time: Option<Instant>,
}

impl HoldTime {
    pub fn new(duration: Duration) -> Self {
        Self { duration, start_time: None }
    }
}

impl Debounce for HoldTime {
    fn on_event(&mut self, now: Instant) -> bool {
        if self.start_time.is_none() {
            self.start_time = Some(now);
            return false;
        }
        now.duration_since(self.start_time.unwrap()) >= self.duration
    }

    fn reset(&mut self, _now: Instant) {
        self.start_time = None;
    }
}

pub struct EdgeWithCooldown {
    cooldown: Duration,
    last_report: Option<Instant>,
}

impl EdgeWithCooldown {
    pub fn new(cooldown: Duration) -> Self {
        Self { cooldown, last_report: None }
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

    fn reset(&mut self, now: Instant) {
        self.last_report = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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
            window: Duration::from_secs(3),
        }
        .into();
        let d2 = DebounceMode::HoldTime {
            duration: Duration::from_secs(1),
        }
        .into();
        let d3 = DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_secs(10),
        }
        .into();

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
                duration: Duration::from_secs(2),
            },
            log_throttle: Some(Duration::from_secs(10)),
        };
        let p2 = p1.clone();
        assert_eq!(p1, p2);
        assert!(format!("{:?}", p1).contains("HoldTime"));
    }
}
