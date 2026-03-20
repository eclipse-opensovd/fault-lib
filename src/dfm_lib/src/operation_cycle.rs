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

//! Tracks named operation cycles for fault aging.
//!
//! Operation cycles are logical time units (e.g., "ignition", "drive", "power")
//! that govern when faults can be automatically reset/aged.
//!
//! # Architecture
//!
//! External lifecycle events (ECU power-on, ignition, etc.) are delivered
//! through an [`OperationCycleProvider`] abstraction. Each provider translates
//! platform-specific signals into [`OperationCycleEvent`] values that the
//! [`OperationCycleTracker`] consumes to advance counters.

use std::{collections::HashMap, time::SystemTime};

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Identifies the source of an operation-cycle event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CycleSource {
    /// Event originates from ECU power management.
    Ecu,
    /// Event originates from HPC lifecycle service.
    Hpc,
    /// Manually injected (e.g. diagnostic tool or test harness).
    Manual,
}

/// What happened with the named operation cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CycleEventType {
    /// A new cycle has started — counter is incremented.
    Start,
    /// The current cycle has ended (informational — counter is not changed).
    Stop,
    /// The cycle was restarted (equivalent to Stop + Start, single increment).
    Restart,
}

/// A typed operation-cycle event.
///
/// Providers emit these events; the tracker consumes them.
#[derive(Debug, Clone)]
pub struct OperationCycleEvent {
    /// Name of the cycle (e.g. "power", "ignition", "drive").
    pub cycle_id: String,
    /// What happened.
    pub event_type: CycleEventType,
    /// When the event occurred.
    pub timestamp: SystemTime,
    /// Where the event came from.
    pub source: CycleSource,
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Abstraction over external operation-cycle event sources.
///
/// Implementors translate platform signals (IPC, CAN, GPIO, etc.)
/// into a stream of [`OperationCycleEvent`] values that the DFM polls.
pub trait OperationCycleProvider: Send {
    /// Drain all pending events since the last call.
    ///
    /// Must be non-blocking. Returns an empty `Vec` when no events are
    /// available. Implementations must be idempotent — calling `poll()`
    /// twice without new external events yields an empty result the second
    /// time.
    fn poll(&mut self) -> Vec<OperationCycleEvent>;

    /// Current count for the named cycle, if the provider tracks it.
    ///
    /// Returns `None` when the cycle name is unknown or the provider
    /// does not keep running totals (in that case the tracker is the
    /// authoritative source).
    fn current_cycle(&self, _cycle_id: &str) -> Option<u64> {
        None
    }
}

/// Tracks operation cycle counts for fault aging logic.
///
/// Each counter is identified by a string reference (e.g., "power", "ignition").
/// External lifecycle events (power-on, ignition, etc.) should call `increment()`
/// to advance the appropriate counter.
#[derive(Debug, Default)]
pub struct OperationCycleTracker {
    cycles: HashMap<String, u64>,
}

impl OperationCycleTracker {
    /// Create a new empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the named cycle counter. Returns the new count.
    ///
    /// # Examples
    /// ```
    /// use dfm_lib::OperationCycleTracker;
    /// let mut tracker = OperationCycleTracker::new();
    /// assert_eq!(tracker.increment("power"), 1);
    /// assert_eq!(tracker.increment("power"), 2);
    /// ```
    pub fn increment(&mut self, cycle_ref: &str) -> u64 {
        let count = self.cycles.entry(cycle_ref.to_string()).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    /// Get current count for a cycle reference. Returns 0 if unknown.
    #[must_use]
    pub fn get(&self, cycle_ref: &str) -> u64 {
        self.cycles.get(cycle_ref).copied().unwrap_or(0)
    }

    /// Snapshot all current cycle values.
    /// Returns a `HashMap` that can be stored with fault aging state.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<String, u64> {
        self.cycles.clone()
    }

    /// Apply a batch of operation-cycle events.
    ///
    /// `Start` and `Restart` increment the named counter; `Stop` is
    /// recorded but does not change the counter (it is informational).
    /// Duplicate events (same `cycle_id` + same resulting count) are
    /// effectively idempotent because `increment()` always moves forward.
    ///
    /// Returns the set of cycle names that were actually incremented.
    pub fn apply_events(&mut self, events: &[OperationCycleEvent]) -> Vec<String> {
        let mut incremented = Vec::new();
        for event in events {
            match event.event_type {
                CycleEventType::Start | CycleEventType::Restart => {
                    self.increment(&event.cycle_id);
                    incremented.push(event.cycle_id.clone());
                }
                CycleEventType::Stop => {
                    // Informational — no counter change.
                }
            }
        }
        incremented
    }
}

// ---------------------------------------------------------------------------
// Mock provider (available in tests and as a manual/debug provider)
// ---------------------------------------------------------------------------

/// A provider backed by an in-memory queue of events.
///
/// Useful for tests and for manual / diagnostic-tool injection.
#[derive(Debug, Default)]
pub struct ManualCycleProvider {
    pending: Vec<OperationCycleEvent>,
}

impl ManualCycleProvider {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a single event that will be returned on the next `poll()`.
    pub fn push(&mut self, event: OperationCycleEvent) {
        self.pending.push(event);
    }

    /// Convenience: enqueue a `Start` event for the given cycle name.
    pub fn start_cycle(&mut self, cycle_id: &str) {
        self.pending.push(OperationCycleEvent {
            cycle_id: cycle_id.to_string(),
            event_type: CycleEventType::Start,
            timestamp: SystemTime::now(),
            source: CycleSource::Manual,
        });
    }

    /// Convenience: enqueue a `Stop` event for the given cycle name.
    pub fn stop_cycle(&mut self, cycle_id: &str) {
        self.pending.push(OperationCycleEvent {
            cycle_id: cycle_id.to_string(),
            event_type: CycleEventType::Stop,
            timestamp: SystemTime::now(),
            source: CycleSource::Manual,
        });
    }
}

impl OperationCycleProvider for ManualCycleProvider {
    fn poll(&mut self) -> Vec<OperationCycleEvent> {
        core::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_returns_zero() {
        let tracker = OperationCycleTracker::new();
        assert_eq!(tracker.get("power"), 0);
        assert_eq!(tracker.get("ignition"), 0);
    }

    #[test]
    fn increment_increases_count() {
        let mut tracker = OperationCycleTracker::new();
        assert_eq!(tracker.increment("power"), 1);
        assert_eq!(tracker.increment("power"), 2);
        assert_eq!(tracker.increment("power"), 3);
        assert_eq!(tracker.get("power"), 3);
    }

    #[test]
    fn independent_cycle_refs() {
        let mut tracker = OperationCycleTracker::new();
        tracker.increment("power");
        tracker.increment("power");
        tracker.increment("ignition");

        assert_eq!(tracker.get("power"), 2);
        assert_eq!(tracker.get("ignition"), 1);
        assert_eq!(tracker.get("drive"), 0);
    }

    #[test]
    fn snapshot_captures_all_cycles() {
        let mut tracker = OperationCycleTracker::new();
        tracker.increment("power");
        tracker.increment("drive");
        tracker.increment("drive");

        let snap = tracker.snapshot();
        assert_eq!(snap.get("power"), Some(&1));
        assert_eq!(snap.get("drive"), Some(&2));
    }

    // -----------------------------------------------------------------------
    // OperationCycleEvent / apply_events tests
    // -----------------------------------------------------------------------

    fn make_event(cycle_id: &str, event_type: CycleEventType) -> OperationCycleEvent {
        OperationCycleEvent {
            cycle_id: cycle_id.to_string(),
            event_type,
            timestamp: SystemTime::now(),
            source: CycleSource::Ecu,
        }
    }

    #[test]
    fn apply_events_start_increments_counter() {
        let mut tracker = OperationCycleTracker::new();
        let events = vec![
            make_event("power", CycleEventType::Start),
            make_event("power", CycleEventType::Start),
        ];
        let incremented = tracker.apply_events(&events);
        assert_eq!(tracker.get("power"), 2);
        assert_eq!(incremented.len(), 2);
    }

    #[test]
    fn apply_events_stop_does_not_increment() {
        let mut tracker = OperationCycleTracker::new();
        tracker.increment("power"); // count = 1
        let events = vec![make_event("power", CycleEventType::Stop)];
        let incremented = tracker.apply_events(&events);
        assert_eq!(tracker.get("power"), 1); // unchanged
        assert!(incremented.is_empty());
    }

    #[test]
    fn apply_events_restart_increments_once() {
        let mut tracker = OperationCycleTracker::new();
        let events = vec![make_event("ignition", CycleEventType::Restart)];
        let incremented = tracker.apply_events(&events);
        assert_eq!(tracker.get("ignition"), 1);
        assert_eq!(incremented, vec!["ignition"]);
    }

    #[test]
    fn apply_events_mixed_sequence() {
        let mut tracker = OperationCycleTracker::new();
        let events = vec![
            make_event("power", CycleEventType::Start),
            make_event("power", CycleEventType::Stop),
            make_event("power", CycleEventType::Start),
            make_event("ignition", CycleEventType::Start),
        ];
        tracker.apply_events(&events);
        assert_eq!(tracker.get("power"), 2);
        assert_eq!(tracker.get("ignition"), 1);
    }

    // -----------------------------------------------------------------------
    // ManualCycleProvider tests
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn manual_provider_poll_drains_queue() {
        let mut provider = ManualCycleProvider::new();
        provider.start_cycle("power");
        provider.start_cycle("ignition");

        let events = provider.poll();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].cycle_id, "power");
        assert_eq!(events[1].cycle_id, "ignition");

        // Second poll is empty (idempotent)
        let events2 = provider.poll();
        assert!(events2.is_empty());
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn manual_provider_start_and_stop() {
        let mut provider = ManualCycleProvider::new();
        provider.start_cycle("drive");
        provider.stop_cycle("drive");

        let events = provider.poll();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, CycleEventType::Start);
        assert_eq!(events[1].event_type, CycleEventType::Stop);
    }

    #[test]
    fn manual_provider_current_cycle_returns_none() {
        let provider = ManualCycleProvider::new();
        assert_eq!(provider.current_cycle("power"), None);
    }

    // -----------------------------------------------------------------------
    // Integration: provider → tracker
    // -----------------------------------------------------------------------

    #[test]
    fn provider_events_drive_tracker() {
        let mut provider = ManualCycleProvider::new();
        provider.start_cycle("power");
        provider.start_cycle("power");
        provider.stop_cycle("power");
        provider.start_cycle("ignition");

        let mut tracker = OperationCycleTracker::new();
        let events = provider.poll();
        tracker.apply_events(&events);

        assert_eq!(tracker.get("power"), 2);
        assert_eq!(tracker.get("ignition"), 1);
    }
}
