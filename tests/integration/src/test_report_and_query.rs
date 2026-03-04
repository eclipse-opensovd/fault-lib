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
//! Basic report → process → query flow.
//!
//! Demonstrates the primary use case: a reporter detects a fault condition,
//! publishes a FaultRecord, DFM processes it, and the SOVD manager can
//! query the resulting fault status.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::helpers::*;
use common::fault::*;
use common::types::*;
use serial_test::serial;

/// **Scenario**: Reporter reports a single fault, DFM stores it, SOVD returns it.
///
/// Steps:
/// 1. Build HVAC catalog with known fault descriptors
/// 2. Reporter creates a `Failed` record for fault 0x7001
/// 3. DFM processor ingests the record
/// 4. SOVD manager returns the fault with `test_failed=true`, `confirmed_dtc=true`
#[test]
#[serial]
fn report_single_fault_and_query_via_sovd() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // Reporter side: create and send a Failed record.
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    // SOVD query side: retrieve all faults for HVAC.
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(faults.len(), 2, "HVAC catalog has 2 registered faults");

    // Find the fault we reported.
    let cabin_temp = faults
        .iter()
        .find(|f| f.code == "0x7001")
        .expect("fault 0x7001 should exist");

    assert_eq!(cabin_temp.fault_name, "CabinTempSensorStuck");
    assert_eq!(cabin_temp.severity, FaultSeverity::Error as u32);

    let status = cabin_temp
        .typed_status
        .as_ref()
        .expect("typed_status should be populated");
    assert_eq!(
        status.test_failed,
        Some(true),
        "Failed stage sets test_failed"
    );
    assert_eq!(
        status.confirmed_dtc,
        Some(true),
        "Failed stage sets confirmed_dtc"
    );
}

/// **Scenario**: Reporter reports a fault with environment data, SOVD returns it.
///
/// Environment data (e.g., sensor readings, temperatures) provides diagnostic
/// context for troubleshooting. This test verifies env_data flows through the
/// full pipeline.
#[test]
#[serial]
fn report_fault_with_env_data_and_query() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    let record = make_fault_record_with_env(
        FaultId::Text(to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap()),
        LifecycleStage::Failed,
        &[("cabin_temp_c", "42"), ("target_temp_c", "22")],
    );
    harness.processor.process_record(&path, &record);

    // Query individual fault with env_data.
    let (fault, env_data) = harness
        .manager
        .get_fault("hvac", "hvac.blower.speed_sensor_mismatch")
        .unwrap();

    assert_eq!(fault.fault_name, "BlowerSpeedMismatch");
    assert_eq!(
        fault.symptom.as_deref(),
        Some("Blower motor speed does not match commanded value")
    );

    // Verify env_data round-trips through the pipeline.
    assert_eq!(env_data.get("cabin_temp_c"), Some(&"42".to_string()));
    assert_eq!(env_data.get("target_temp_c"), Some(&"22".to_string()));
}

/// **Scenario**: Querying an unknown catalog path returns an error.
///
/// The SOVD manager should reject queries for paths not registered
/// in the fault catalog registry.
#[test]
#[serial]
fn query_unknown_path_returns_error() {
    let harness = TestHarness::new(vec![hvac_catalog_config()]);

    let result = harness.manager.get_all_faults("nonexistent_ecu");
    assert!(result.is_err(), "Unknown path should return error");
}

/// **Scenario**: Faults that were never reported still appear with default status.
///
/// SOVD specification requires all registered faults to be visible,
/// even if no fault record was ever received for them.
#[test]
#[serial]
fn unreported_faults_have_default_status() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(faults.len(), 2, "Both HVAC faults should be listed");

    for fault in &faults {
        let status = fault
            .typed_status
            .as_ref()
            .expect("typed_status should exist");
        assert_eq!(
            status.test_failed,
            Some(false),
            "Default: test_failed=false"
        );
        assert_eq!(
            status.confirmed_dtc,
            Some(false),
            "Default: confirmed_dtc=false"
        );
        assert_eq!(
            status.pending_dtc,
            Some(false),
            "Default: pending_dtc=false"
        );
    }
}
