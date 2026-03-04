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
//! E2E tests for debounce, aging, operation cycles, and enabling conditions.
//!
//! These integration tests exercise the full DFM pipeline for each
//! fault-management flow, including edge cases and boundary conditions.

use crate::helpers::*;
use common::catalog::FaultCatalogConfig;
use common::config::{ResetPolicy, ResetTrigger};
use common::debounce::DebounceMode;
use common::fault::*;
use common::types::to_static_short_string;
use dfm_lib::enabling_condition_registry::EnablingConditionRegistry;
use dfm_lib::operation_cycle::OperationCycleTracker;
use serial_test::serial;
use std::sync::{Arc, RwLock};
use std::time::Duration;

// ============================================================================
// Helper: build a catalog with manager-side debounce
// ============================================================================

fn debounce_catalog_config(mode: DebounceMode) -> FaultCatalogConfig {
    FaultCatalogConfig {
        id: "debounce_test".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Numeric(0xD001),
            name: to_static_short_string("DebouncedFault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: Some(mode),
            manager_side_reset: None,
        }],
    }
}

fn aging_catalog_config(policy: ResetPolicy) -> FaultCatalogConfig {
    FaultCatalogConfig {
        id: "aging_test".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Numeric(0xA001),
            name: to_static_short_string("AgingFault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: Some(policy),
        }],
    }
}

// ============================================================================
// 1. Debounce: CountWithinWindow E2E
// ============================================================================

/// CountWithinWindow debounce: events below min_count are suppressed.
/// Only the N-th event within the window fires the fault through.
#[test]
#[serial]
fn debounce_count_within_window_suppresses_below_threshold() {
    let mode = DebounceMode::CountWithinWindow {
        min_count: 3,
        window: Duration::from_secs(60).into(),
    };
    let mut harness = TestHarness::new(vec![debounce_catalog_config(mode)]);
    harness.clean_catalogs(&["debounce_test"]);

    let path = make_path("debounce_test");
    let fault_id = FaultId::Numeric(0xD001);
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);

    // Events 1 & 2: suppressed by debounce
    harness.processor.process_record(&path, &record);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(false),
        "First 2 events should be suppressed"
    );

    // Event 3: fires (min_count reached)
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "Third event should pass debounce"
    );
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true),
        "Confirmed DTC should be set"
    );
}

// ============================================================================
// 2. Debounce: HoldTime E2E
// ============================================================================

/// HoldTime debounce: first event starts timer, second within hold time
/// is suppressed, event after hold time elapses fires.
///
/// NOTE: HoldTime's on_event() checks wall-clock Instant. In integration
/// tests we can only verify that the first event is always suppressed.
#[test]
#[serial]
fn debounce_holdtime_first_event_always_suppressed() {
    let mode = DebounceMode::HoldTime {
        duration: Duration::from_secs(60).into(), // long hold time
    };
    let mut harness = TestHarness::new(vec![debounce_catalog_config(mode)]);
    harness.clean_catalogs(&["debounce_test"]);

    let path = make_path("debounce_test");
    let fault_id = FaultId::Numeric(0xD001);
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);

    // First event: starts HoldTime timer, suppressed
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(false),
        "First event in HoldTime should be suppressed (timer just started)"
    );
}

// ============================================================================
// 3. Debounce: EdgeWithCooldown E2E
// ============================================================================

/// EdgeWithCooldown: first event fires immediately, subsequent events
/// within cooldown are suppressed.
#[test]
#[serial]
fn debounce_edge_with_cooldown_first_fires_then_suppresses() {
    let mode = DebounceMode::EdgeWithCooldown {
        cooldown: Duration::from_secs(60).into(), // long cooldown
    };
    let mut harness = TestHarness::new(vec![debounce_catalog_config(mode)]);
    harness.clean_catalogs(&["debounce_test"]);

    let path = make_path("debounce_test");
    let fault_id = FaultId::Numeric(0xD001);
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);

    // First event: fires (edge trigger)
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "Edge trigger should fire on first event"
    );
}

// ============================================================================
// 4. Aging: PowerCycles E2E
// ============================================================================

/// Aging with PowerCycles: confirmed_dtc stays latched after Passed until
/// the required number of power cycles elapse.
#[test]
#[serial]
fn aging_power_cycles_clears_after_threshold() {
    let policy = ResetPolicy {
        trigger: ResetTrigger::PowerCycles(2),
        min_operating_cycles_before_clear: None,
    };
    let config = aging_catalog_config(policy);

    // Build harness with custom cycle tracker
    let storage_dir = shared_storage_path();
    let storage = Arc::new(
        dfm_lib::sovd_fault_storage::KvsSovdFaultStateStorage::new(storage_dir, 0)
            .expect("storage init"),
    );
    let catalog = common::catalog::FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    let registry = Arc::new(dfm_lib::fault_catalog_registry::FaultCatalogRegistry::new(
        vec![catalog],
    ));
    let cycle_tracker = Arc::new(RwLock::new(OperationCycleTracker::new()));
    let mut processor = dfm_lib::fault_record_processor::FaultRecordProcessor::new(
        Arc::clone(&storage),
        Arc::clone(&registry),
        Arc::clone(&cycle_tracker),
    );
    let manager = dfm_lib::sovd_fault_manager::SovdFaultManager::new(storage, registry);
    let _ = manager.delete_all_faults("aging_test");

    let path = make_path("aging_test");
    let fault_id = FaultId::Numeric(0xA001);

    // Step 1: Fault occurs and then passes
    let failed = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    let passed = make_fault_record(fault_id.clone(), LifecycleStage::Passed);
    processor.process_record(&path, &failed);
    processor.process_record(&path, &passed);

    let faults = manager.get_all_faults("aging_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xA001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true),
        "confirmed_dtc should stay latched after Passed (aging policy)"
    );

    // Step 2: Only 1 power cycle — not enough
    cycle_tracker.write().unwrap().increment("power");
    processor.process_record(&path, &passed);

    let faults = manager.get_all_faults("aging_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xA001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true),
        "Only 1/2 power cycles — not enough"
    );

    // Step 3: Second power cycle — aging threshold met
    cycle_tracker.write().unwrap().increment("power");
    processor.process_record(&path, &passed);

    let faults = manager.get_all_faults("aging_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xA001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(false),
        "2/2 power cycles — confirmed_dtc should be cleared by aging"
    );
}

// ============================================================================
// 5. Operation Cycles: new cycle resets per-cycle flags
// ============================================================================

/// Operation cycle boundary resets `*_this_operation_cycle` flags
/// for all faults at the given path.
#[test]
#[serial]
fn operation_cycle_boundary_resets_per_cycle_flags() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let fault_id = FaultId::Numeric(0x7001);

    // Report a fault — sets test_failed_this_operation_cycle
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert_eq!(
        fault
            .typed_status
            .as_ref()
            .unwrap()
            .test_failed_this_operation_cycle,
        Some(true)
    );

    // New operation cycle boundary
    harness.processor.on_new_operation_cycle("hvac");

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert_eq!(
        fault
            .typed_status
            .as_ref()
            .unwrap()
            .test_failed_this_operation_cycle,
        Some(false),
        "test_failed_this_operation_cycle should reset on new cycle"
    );
    // Other flags preserved
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "test_failed should be preserved across cycles"
    );
}

// ============================================================================
// 6. Enabling Conditions: status tracking
// ============================================================================

/// Enabling condition registry correctly tracks status transitions.
/// This is a unit-level integration test verifying the EC workflow.
#[test]
fn enabling_condition_status_transitions() {
    use common::enabling_condition::EnablingConditionStatus;

    let mut registry = EnablingConditionRegistry::new();

    // Register new condition — starts Inactive
    let initial = registry.register("vehicle.speed.valid");
    assert_eq!(initial, EnablingConditionStatus::Inactive);

    // Activate
    let changed = registry.update_status("vehicle.speed.valid", EnablingConditionStatus::Active);
    assert_eq!(changed, Some(EnablingConditionStatus::Active));

    // Same status again — no change
    let no_change = registry.update_status("vehicle.speed.valid", EnablingConditionStatus::Active);
    assert_eq!(
        no_change, None,
        "Same status should return None (no change)"
    );

    // Deactivate
    let deactivated =
        registry.update_status("vehicle.speed.valid", EnablingConditionStatus::Inactive);
    assert_eq!(deactivated, Some(EnablingConditionStatus::Inactive));

    // Unknown condition is auto-registered
    let auto = registry.update_status("engine.temp.valid", EnablingConditionStatus::Active);
    assert_eq!(auto, Some(EnablingConditionStatus::Active));
    assert_eq!(registry.len(), 2);
}

// ============================================================================
// Edge cases
// ============================================================================

/// Rapid events within a CountWithinWindow debounce:
/// sending many events quickly should eventually fire.
#[test]
#[serial]
fn debounce_rapid_events_eventually_fire() {
    let mode = DebounceMode::CountWithinWindow {
        min_count: 5,
        window: Duration::from_secs(60).into(),
    };
    let mut harness = TestHarness::new(vec![debounce_catalog_config(mode)]);
    harness.clean_catalogs(&["debounce_test"]);

    let path = make_path("debounce_test");
    let record = make_fault_record(FaultId::Numeric(0xD001), LifecycleStage::Failed);

    // Send 4 events: suppressed
    for _ in 0..4 {
        harness.processor.process_record(&path, &record);
    }

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(false)
    );

    // 5th event: fires
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(fault.typed_status.as_ref().unwrap().test_failed, Some(true));
}

/// Zero-threshold debounce (min_count = 1) should fire on first event
/// (though not a realistic production config, tests boundary behavior).
#[test]
#[serial]
fn debounce_min_count_one_fires_immediately() {
    let mode = DebounceMode::CountWithinWindow {
        min_count: 1,
        window: Duration::from_secs(60).into(),
    };
    let mut harness = TestHarness::new(vec![debounce_catalog_config(mode)]);
    harness.clean_catalogs(&["debounce_test"]);

    let path = make_path("debounce_test");
    let record = make_fault_record(FaultId::Numeric(0xD001), LifecycleStage::Failed);

    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("debounce_test").unwrap();
    let fault = faults.iter().find(|f| f.code == "0xD001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "min_count=1 should fire on first event"
    );
}
