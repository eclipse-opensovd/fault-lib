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
//! Fault lifecycle transition tests.
//!
//! Validates the complete fault lifecycle as seen through the SOVD interface:
//! NotTested → PreFailed → Failed → PrePassed → Passed, and back.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::helpers::*;
use common::fault::*;
use serial_test::serial;

/// **Scenario**: Full lifecycle: NotTested → Failed → Passed.
///
/// A newly detected fault transitions through the standard lifecycle.
/// After being reported as `Failed`, the DTC flags are set. When the
/// fault clears (`Passed`), the flags reset.
#[test]
#[serial]
fn full_lifecycle_failed_then_passed() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // Step 1: Report fault as Failed.
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();
    assert_eq!(status.test_failed, Some(true));
    assert_eq!(status.confirmed_dtc, Some(true));

    // Step 2: Fault clears — report as Passed.
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Passed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();
    assert_eq!(status.test_failed, Some(false), "Passed clears test_failed");
    assert_eq!(
        status.confirmed_dtc,
        Some(false),
        "Passed clears confirmed_dtc (no aging policy)"
    );
}

/// **Scenario**: Pre-stages set pending DTC without confirming.
///
/// `PreFailed` indicates the fault condition is developing but not yet
/// confirmed. The `pending_dtc` flag is set, but `confirmed_dtc` remains
/// false until a `Failed` event arrives.
#[test]
#[serial]
fn prefailed_sets_pending_without_confirming() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::PreFailed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();

    assert_eq!(status.test_failed, Some(true), "PreFailed sets test_failed");
    assert_eq!(status.pending_dtc, Some(true), "PreFailed sets pending_dtc");
    assert_eq!(
        status.confirmed_dtc,
        Some(false),
        "PreFailed does NOT set confirmed_dtc"
    );
}

/// **Scenario**: PreFailed → Failed confirms the DTC.
///
/// The standard diagnostic flow: a developing fault (PreFailed) is
/// confirmed (Failed). The pending flag clears and confirmed is set.
#[test]
#[serial]
fn prefailed_then_failed_confirms_dtc() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // PreFailed
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::PreFailed);
    harness.processor.process_record(&path, &record);

    // Failed (confirms)
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();

    assert_eq!(status.test_failed, Some(true));
    assert_eq!(status.pending_dtc, Some(false), "Failed clears pending_dtc");
    assert_eq!(
        status.confirmed_dtc,
        Some(true),
        "Failed sets confirmed_dtc"
    );
}

/// **Scenario**: NotTested marks the test-not-completed flag.
///
/// When a diagnostic monitor cannot complete (e.g., preconditions not met),
/// it reports `NotTested`. This sets the ISO 14229 bit for
/// "test not completed this operation cycle".
#[test]
#[serial]
fn not_tested_sets_incomplete_flag() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::NotTested);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();

    assert_eq!(
        status.test_not_completed_this_operation_cycle,
        Some(true),
        "NotTested sets test_not_completed_this_operation_cycle"
    );
}

/// **Scenario**: Intermittent fault — Failed → Passed → Failed again.
///
/// An intermittent fault clears and then re-occurs. The SOVD status
/// should reflect the latest state after each transition.
#[test]
#[serial]
fn intermittent_fault_toggles_correctly() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // First occurrence
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);
    let fault = harness
        .manager
        .get_all_faults("hvac")
        .unwrap()
        .into_iter()
        .find(|f| f.code == "0x7001")
        .unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true)
    );

    // Clears
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Passed);
    harness.processor.process_record(&path, &record);
    let fault = harness
        .manager
        .get_all_faults("hvac")
        .unwrap()
        .into_iter()
        .find(|f| f.code == "0x7001")
        .unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(false)
    );

    // Re-occurs
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);
    let fault = harness
        .manager
        .get_all_faults("hvac")
        .unwrap()
        .into_iter()
        .find(|f| f.code == "0x7001")
        .unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true),
        "Re-confirmed after re-occurrence"
    );
    assert_eq!(fault.typed_status.as_ref().unwrap().test_failed, Some(true));
}
