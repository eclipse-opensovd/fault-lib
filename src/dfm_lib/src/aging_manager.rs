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

//! Evaluates fault aging policies and determines when faults should be reset.
//!
//! Aging logic checks whether a fault has been stable long enough
//! (either in operation cycles or wall-clock time) to be cleared.

use crate::operation_cycle::OperationCycleTracker;
use crate::sovd_fault_storage::SovdFaultState;
use alloc::sync::Arc;
use common::config::{ResetPolicy, ResetTrigger};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

/// Per-fault aging state (runtime-only, **not persisted** across restarts).
///
/// Tracks when a fault was last active and at what cycle counts,
/// enabling the aging manager to determine if reset conditions are met.
///
/// # Persistence caveat
///
/// `last_active_cycle` and `is_healed` live only in process memory.
/// On DFM restart, aging progress is lost: a fault that was 4/5 through
/// its aging window restarts from 0. The cumulative `aging_counter` and
/// `healing_counter` ARE persisted via `SovdFaultState` in KVS, so the
/// total count survives restarts - only the in-progress window is reset.
#[derive(Debug, Clone)]
pub struct AgingState {
    /// Cycle count at which fault last occurred, keyed by `cycle_ref`.
    pub last_active_cycle: HashMap<String, u64>,
    /// Timestamp of last active (Failed/PreFailed) state.
    pub last_active_time: Instant,
    /// Number of times aging/reset was applied to this fault.
    pub aging_counter: u32,
    /// Whether the fault is currently in healed state.
    pub is_healed: bool,
}

impl AgingState {
    /// Create a new aging state for a fault that just became active.
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_active_cycle: HashMap::new(),
            last_active_time: Instant::now(),
            aging_counter: 0,
            is_healed: false,
        }
    }

    /// Mark fault as active (resets aging progress).
    /// Call when fault transitions to Failed or `PreFailed`.
    pub fn mark_active(&mut self, cycle_tracker: &OperationCycleTracker) {
        self.last_active_time = Instant::now();
        self.is_healed = false;
        // Snapshot current cycles
        self.last_active_cycle = cycle_tracker.snapshot();
    }

    /// Mark fault as healed after reset policy triggered.
    pub fn mark_healed(&mut self) {
        self.aging_counter = self.aging_counter.saturating_add(1);
        self.is_healed = true;
    }
}

impl Default for AgingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluates aging (reset) policies for faults.
///
/// The aging manager holds a reference to the operation cycle tracker
/// and can evaluate whether a fault's reset policy conditions are met.
pub struct AgingManager {
    cycle_tracker: Arc<RwLock<OperationCycleTracker>>,
}

impl AgingManager {
    /// Create an aging manager with a shared cycle tracker.
    pub fn new(cycle_tracker: Arc<RwLock<OperationCycleTracker>>) -> Self {
        Self { cycle_tracker }
    }

    /// Check if the given reset policy is satisfied for this aging state.
    /// Returns `true` if the fault should be reset (healed).
    pub fn should_reset(&self, policy: &ResetPolicy, state: &AgingState) -> bool {
        // Already healed faults don't need re-evaluation
        if state.is_healed {
            return false;
        }

        // ISO 14229: some regulations require a minimum number of operation
        // cycles before a fault may be cleared. Gate the trigger evaluation
        // behind this threshold when configured.
        if let Some(min_cycles) = policy.min_operating_cycles_before_clear {
            let tracker = self
                .cycle_tracker
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current_power = tracker.get("power");
            let fault_power = state.last_active_cycle.get("power").copied().unwrap_or(0);
            if current_power.saturating_sub(fault_power) < u64::from(min_cycles) {
                return false;
            }
        }

        self.evaluate_trigger(&policy.trigger, state)
    }

    fn evaluate_trigger(&self, trigger: &ResetTrigger, state: &AgingState) -> bool {
        match trigger {
            ResetTrigger::PowerCycles(min_cycles) => {
                // PowerCycles uses the "power" counter for backward compatibility.
                // Recover from RwLock poisoning — data integrity is more important
                // than propagating a panic from an unrelated thread.
                let tracker = self
                    .cycle_tracker
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let current = tracker.get("power");
                let fault_cycle = state.last_active_cycle.get("power").copied().unwrap_or(0);
                current.saturating_sub(fault_cycle) >= u64::from(*min_cycles)
            }

            ResetTrigger::OperationCycles {
                min_cycles,
                cycle_ref,
            } => {
                let tracker = self
                    .cycle_tracker
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let ref_str = cycle_ref.to_string();
                let current = tracker.get(&ref_str);
                let fault_cycle = state.last_active_cycle.get(&ref_str).copied().unwrap_or(0);
                current.saturating_sub(fault_cycle) >= u64::from(*min_cycles)
            }

            ResetTrigger::StableFor(duration) => {
                Instant::now().duration_since(state.last_active_time) >= (*duration).into()
            }

            ResetTrigger::ToolOnly => false, // Never auto-reset
        }
    }

    /// Apply reset to fault state, clearing DTC flags.
    /// Call this when `should_reset()` returns `true`.
    pub fn apply_reset(&self, aging_state: &mut AgingState, sovd_state: &mut SovdFaultState) {
        aging_state.mark_healed();

        // Clear DTC flags in the SOVD state
        sovd_state.confirmed_dtc = false;
        sovd_state.pending_dtc = false;
        sovd_state.test_failed = false;
        sovd_state.warning_indicator_requested = false;

        // Increment persisted counters directly (saturating to prevent overflow).
        // Using saturating_add on sovd_state rather than syncing from in-memory
        // aging_state ensures correctness across process restarts where
        // aging_state starts at 0 but sovd_state is loaded from KVS.
        sovd_state.aging_counter = sovd_state.aging_counter.saturating_add(1);
        sovd_state.healing_counter = sovd_state.healing_counter.saturating_add(1);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use common::types::ShortString;
    use std::time::Duration;

    fn make_tracker() -> Arc<RwLock<OperationCycleTracker>> {
        Arc::new(RwLock::new(OperationCycleTracker::new()))
    }

    #[test]
    fn power_cycles_trigger_not_met() {
        let tracker = make_tracker();
        let manager = AgingManager::new(Arc::clone(&tracker));

        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(3),
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        state.mark_active(&tracker.read().unwrap());

        // Only 2 power cycles — should not reset
        tracker.write().unwrap().increment("power");
        tracker.write().unwrap().increment("power");

        assert!(!manager.should_reset(&policy, &state));
    }

    #[test]
    fn power_cycles_trigger_met() {
        let tracker = make_tracker();
        let manager = AgingManager::new(Arc::clone(&tracker));

        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(3),
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        state.mark_active(&tracker.read().unwrap());

        // 3 power cycles — should reset
        for _ in 0..3 {
            tracker.write().unwrap().increment("power");
        }

        assert!(manager.should_reset(&policy, &state));
    }

    #[test]
    fn operation_cycles_named_counter() {
        let tracker = make_tracker();
        let manager = AgingManager::new(Arc::clone(&tracker));

        let policy = ResetPolicy {
            trigger: ResetTrigger::OperationCycles {
                min_cycles: 2,
                cycle_ref: ShortString::try_from("ignition").unwrap(),
            },
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        state.mark_active(&tracker.read().unwrap());

        // Increment different counter — should not affect ignition trigger
        tracker.write().unwrap().increment("power");
        tracker.write().unwrap().increment("power");
        assert!(!manager.should_reset(&policy, &state));

        // Increment ignition counter
        tracker.write().unwrap().increment("ignition");
        assert!(!manager.should_reset(&policy, &state)); // Only 1

        tracker.write().unwrap().increment("ignition");
        assert!(manager.should_reset(&policy, &state)); // Now 2
    }

    #[test]
    fn stable_for_trigger() {
        let tracker = make_tracker();
        let manager = AgingManager::new(tracker);

        let policy = ResetPolicy {
            trigger: ResetTrigger::StableFor(Duration::from_millis(10).into()),
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        // State was just marked active — not enough time passed
        assert!(!manager.should_reset(&policy, &state));

        // Backdate the last_active_time
        state.last_active_time = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .unwrap();
        assert!(manager.should_reset(&policy, &state));
    }

    #[test]
    fn tool_only_never_auto_resets() {
        let tracker = make_tracker();
        let manager = AgingManager::new(Arc::clone(&tracker));

        let policy = ResetPolicy {
            trigger: ResetTrigger::ToolOnly,
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        state.mark_active(&tracker.read().unwrap());

        // Even after 100 cycles, should not reset
        for _ in 0..100 {
            tracker.write().unwrap().increment("power");
        }

        assert!(!manager.should_reset(&policy, &state));
    }

    #[test]
    fn apply_reset_clears_flags() {
        let tracker = make_tracker();
        let manager = AgingManager::new(tracker);

        let mut aging_state = AgingState::new();
        let mut sovd_state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            pending_dtc: true,
            warning_indicator_requested: true,
            ..Default::default()
        };

        manager.apply_reset(&mut aging_state, &mut sovd_state);

        assert!(aging_state.is_healed);
        assert_eq!(aging_state.aging_counter, 1);
        assert!(!sovd_state.test_failed);
        assert!(!sovd_state.confirmed_dtc);
        assert!(!sovd_state.pending_dtc);
        assert!(!sovd_state.warning_indicator_requested);
        assert_eq!(sovd_state.aging_counter, 1);
        assert_eq!(sovd_state.healing_counter, 1);
    }

    #[test]
    fn apply_reset_n_cycles_increments_aging_counter() {
        let tracker = make_tracker();
        let manager = AgingManager::new(tracker);

        let mut aging_state = AgingState::new();
        let mut sovd_state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            ..Default::default()
        };

        for expected in 1..=5u32 {
            aging_state.is_healed = false;
            sovd_state.test_failed = true;
            sovd_state.confirmed_dtc = true;
            manager.apply_reset(&mut aging_state, &mut sovd_state);
            assert_eq!(
                sovd_state.aging_counter, expected,
                "after {expected} resets"
            );
            assert_eq!(
                sovd_state.healing_counter, expected,
                "healing_counter after {expected} resets"
            );
        }
    }

    #[test]
    fn already_healed_not_rechecked() {
        let tracker = make_tracker();
        let manager = AgingManager::new(Arc::clone(&tracker));

        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(1),
            min_operating_cycles_before_clear: None,
        };

        let mut state = AgingState::new();
        state.mark_active(&tracker.read().unwrap());
        state.mark_healed(); // Already healed

        tracker.write().unwrap().increment("power");

        // Already healed — should not re-trigger
        assert!(!manager.should_reset(&policy, &state));
    }
}

#[cfg(test)]
mod aging_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use crate::dfm_test_utils::*;
    use crate::fault_record_processor::FaultRecordProcessor;
    use crate::sovd_fault_storage::SovdFaultStateStorage;
    use common::config::{ResetPolicy, ResetTrigger};
    use common::fault::*;
    use common::types::ShortString;
    use std::sync::Arc;
    use std::time::Duration;

    // ============================================================================
    // PowerCycles aging
    // ============================================================================

    /// E2E: fault Failed → Passed → power cycles → aging reset clears `confirmed_dtc`.
    #[test]
    fn aging_power_cycles_clears_confirmed_dtc() {
        let fault_id = FaultId::Numeric(500);
        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(3),
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        // Step 1: Fault occurs
        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &failed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(state.confirmed_dtc, "Failed should set confirmed_dtc");
        assert!(state.test_failed, "Failed should set test_failed");

        // Step 2: Fault passes — confirmed_dtc should stay latched (aging policy)
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "confirmed_dtc should stay latched after Passed (aging policy)"
        );
        assert!(!state.test_failed, "test_failed should clear on Passed");

        // Step 3: Advance power cycles but not enough (only 2 of 3 needed)
        tracker.write().unwrap().increment("power");
        tracker.write().unwrap().increment("power");

        // Send another Passed event to trigger re-evaluation
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "confirmed_dtc should still be latched (only 2/3 power cycles)"
        );

        // Step 4: Third power cycle → aging conditions met
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.confirmed_dtc,
            "confirmed_dtc should clear after 3 power cycles (aging reset)"
        );
        assert!(
            !state.test_failed,
            "test_failed should remain cleared after aging reset"
        );
        assert_eq!(
            state.healing_counter, 1,
            "healing_counter should increment on aging reset"
        );
    }

    // ============================================================================
    // OperationCycles aging (named counter)
    // ============================================================================

    /// E2E: fault resets after N named operation cycles (ignition).
    #[test]
    fn aging_operation_cycles_named_counter_resets() {
        let fault_id = FaultId::Numeric(501);
        let policy = ResetPolicy {
            trigger: ResetTrigger::OperationCycles {
                min_cycles: 2,
                cycle_ref: ShortString::try_from("ignition").unwrap(),
            },
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        // Fault occurs and then passes
        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(state.confirmed_dtc, "confirmed_dtc latched");

        // Increment a different counter — should not affect ignition trigger
        tracker.write().unwrap().increment("power");
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "power cycles should not trigger ignition-based aging"
        );

        // Increment ignition counter: 1 of 2
        tracker.write().unwrap().increment("ignition");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(state.confirmed_dtc, "Only 1/2 ignition cycles — not enough");

        // Increment ignition counter: 2 of 2 — should trigger
        tracker.write().unwrap().increment("ignition");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.confirmed_dtc,
            "2/2 ignition cycles should trigger aging reset"
        );
        assert_eq!(state.healing_counter, 1);
    }

    // ============================================================================
    // StableFor aging (time-based)
    // ============================================================================

    /// E2E: fault resets after being stable (Passed) for a given duration.
    #[test]
    #[cfg_attr(miri, ignore)] // timing-dependent: Miri's ~100x slowdown causes wall-clock drift
    fn aging_stable_for_duration_resets() {
        let fault_id = FaultId::Numeric(502);
        let policy = ResetPolicy {
            trigger: ResetTrigger::StableFor(Duration::from_millis(10).into()),
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor = FaultRecordProcessor::new(Arc::clone(&storage), registry, tracker);
        let path = make_path("test_entity");

        // Fault occurs and passes
        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "confirmed_dtc should be latched immediately after Passed"
        );

        // Wait for the stability duration to elapse
        std::thread::sleep(Duration::from_millis(30));

        // Re-process a Passed event — aging evaluation should now trigger
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.confirmed_dtc,
            "confirmed_dtc should clear after stability period elapsed"
        );
        assert_eq!(state.healing_counter, 1);
    }

    // ============================================================================
    // ToolOnly — never auto-resets
    // ============================================================================

    /// E2E: fault with `ToolOnly` policy never auto-resets.
    #[test]
    fn aging_tool_only_never_auto_resets() {
        let fault_id = FaultId::Numeric(503);
        let policy = ResetPolicy {
            trigger: ResetTrigger::ToolOnly,
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);

        // Even after many cycles, should not auto-reset
        for _ in 0..100 {
            tracker.write().unwrap().increment("power");
        }
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "ToolOnly should never auto-reset — confirmed_dtc stays latched"
        );
        assert_eq!(state.healing_counter, 0);
    }

    // ============================================================================
    // No aging policy — immediate clear on Passed
    // ============================================================================

    /// Without aging policy, `confirmed_dtc` clears immediately on Passed.
    #[test]
    fn no_aging_policy_clears_immediately() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry(); // No aging configured
        let tracker = make_cycle_tracker();
        let mut processor = FaultRecordProcessor::new(Arc::clone(&storage), registry, tracker);
        let path = make_path("test_entity");

        let failed = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        let passed = make_record(FaultId::Numeric(42), LifecycleStage::Passed);
        processor.process_record(&path, &failed);

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(state.confirmed_dtc);

        processor.process_record(&path, &passed);
        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(
            !state.confirmed_dtc,
            "Without aging, confirmed_dtc should clear immediately on Passed"
        );
    }

    // ============================================================================
    // Flapping: re-failure resets aging progress
    // ============================================================================

    /// Fast flapping: if a fault re-fails before aging completes, aging resets.
    #[test]
    fn flapping_resets_aging_progress() {
        let fault_id = FaultId::Numeric(504);
        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(5),
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);

        // First occurrence
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);

        // Advance 3 of 5 power cycles
        for _ in 0..3 {
            tracker.write().unwrap().increment("power");
        }
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "Only 3/5 cycles — should not have aged out yet"
        );

        // Fault re-occurs! Aging resets
        processor.process_record(&path, &failed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "Re-failure keeps confirmed_dtc latched"
        );

        // Now pass again — aging counter should restart from THIS failure
        processor.process_record(&path, &passed);

        // Previous 3 power cycles should NOT count toward the new aging window.
        // Need 5 more power cycles from the new failure point.
        for _ in 0..4 {
            tracker.write().unwrap().increment("power");
        }
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.confirmed_dtc,
            "Only 4/5 cycles since re-failure — should not have aged out"
        );

        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.confirmed_dtc,
            "5/5 cycles since re-failure — aging should complete now"
        );
        assert_eq!(state.healing_counter, 1);
    }

    // ============================================================================
    // Regression: existing status bits preserved
    // ============================================================================

    /// Aging reset preserves `warning_indicator_requested=false` and doesn't
    /// corrupt other status bits.
    #[test]
    fn aging_reset_preserves_status_consistency() {
        let fault_id = FaultId::Numeric(505);
        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(1),
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);

        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);

        // One power cycle — reset triggers
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(!state.confirmed_dtc, "confirmed_dtc cleared");
        assert!(!state.pending_dtc, "pending_dtc cleared");
        assert!(!state.test_failed, "test_failed cleared");
        assert!(!state.warning_indicator_requested, "WIR cleared");
        assert_eq!(state.healing_counter, 1, "healing counter incremented");
    }

    /// Multiple aging resets increment `healing_counter` each time.
    #[test]
    fn multiple_aging_resets_increment_healing_counter() {
        let fault_id = FaultId::Numeric(506);
        let policy = ResetPolicy {
            trigger: ResetTrigger::PowerCycles(1),
            min_operating_cycles_before_clear: None,
        };
        let registry = make_aging_registry("test_entity", fault_id.clone(), policy);
        let storage = Arc::new(InMemoryStorage::new());
        let tracker = make_cycle_tracker();
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&tracker));
        let path = make_path("test_entity");

        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);

        // Cycle 1: fail → pass → age
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert_eq!(state.healing_counter, 1);

        // Cycle 2: re-fail → pass → age
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert_eq!(state.healing_counter, 2);

        // Cycle 3: re-fail → pass → age
        processor.process_record(&path, &failed);
        processor.process_record(&path, &passed);
        tracker.write().unwrap().increment("power");
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert_eq!(state.healing_counter, 3);
    }
}
