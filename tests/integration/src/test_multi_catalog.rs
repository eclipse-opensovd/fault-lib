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
//! Multi-catalog integration tests.
//!
//! Validates that multiple reporter applications (each with their own fault
//! catalog) can coexist within a single DFM instance without interference.
//! This is the primary multi-tenant scenario in a vehicle ECU.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::helpers::*;
use common::fault::*;
use common::types::*;
use serial_test::serial;

/// **Scenario**: Two catalogs (HVAC + IVI) registered, faults are isolated.
///
/// When two subsystems register their catalogs with DFM, fault records
/// from one subsystem must not affect the other. SOVD queries are scoped
/// by the catalog path (e.g., "hvac" or "ivi").
#[test]
#[serial]
fn multi_catalog_fault_isolation() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config(), ivi_catalog_config()]);
    harness.clean_catalogs(&["hvac", "ivi"]);

    // HVAC reporter sends a Failed fault.
    let hvac_path = make_path("hvac");
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&hvac_path, &record);

    // IVI reporter sends a different Failed fault.
    let ivi_path = make_path("ivi");
    let record = make_fault_record(
        FaultId::Text(to_static_short_string("ivi.display.init_timeout").unwrap()),
        LifecycleStage::Failed,
    );
    harness.processor.process_record(&ivi_path, &record);

    // HVAC faults — only HVAC fault should show as failed.
    let hvac_faults = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(hvac_faults.len(), 2, "HVAC catalog has 2 faults");
    let cabin_temp = hvac_faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert_eq!(
        cabin_temp.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true)
    );

    // IVI faults — only IVI fault should show as failed.
    let ivi_faults = harness.manager.get_all_faults("ivi").unwrap();
    assert_eq!(ivi_faults.len(), 1, "IVI catalog has 1 fault");
    let display = &ivi_faults[0];
    assert_eq!(display.code, "ivi.display.init_timeout");
    assert_eq!(
        display.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true)
    );

    // Cross-check: HVAC blower fault should NOT be affected by IVI.
    let blower = hvac_faults
        .iter()
        .find(|f| f.code == "hvac.blower.speed_sensor_mismatch")
        .unwrap();
    assert_eq!(
        blower.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(false),
        "HVAC blower fault should not be affected by IVI fault"
    );
}

/// **Scenario**: Same fault ID in different catalogs stays independent.
///
/// Two subsystems may define the same fault ID (e.g., both use text ID "d1").
/// DFM must keep their states separate because they are scoped by path.
#[test]
#[serial]
fn same_fault_id_different_catalogs_independent() {
    use common::catalog::FaultCatalogConfig;

    // Two catalogs both defining a fault with text ID "comm_error".
    let config_a = FaultCatalogConfig {
        id: "subsystem_a".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Text(to_static_short_string("comm_error").unwrap()),
            name: to_static_short_string("CommErrorA").unwrap(),
            summary: None,
            category: FaultType::Communication,
            severity: FaultSeverity::Error,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let config_b = FaultCatalogConfig {
        id: "subsystem_b".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Text(to_static_short_string("comm_error").unwrap()),
            name: to_static_short_string("CommErrorB").unwrap(),
            summary: None,
            category: FaultType::Communication,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config_a, config_b]);
    harness.clean_catalogs(&["subsystem_a", "subsystem_b"]);

    // Only subsystem_a reports the fault.
    let path_a = make_path("subsystem_a");
    let record = make_fault_record(
        FaultId::Text(to_static_short_string("comm_error").unwrap()),
        LifecycleStage::Failed,
    );
    harness.processor.process_record(&path_a, &record);

    // Subsystem A: comm_error is confirmed.
    let faults_a = harness.manager.get_all_faults("subsystem_a").unwrap();
    assert_eq!(faults_a.len(), 1);
    assert_eq!(faults_a[0].fault_name, "CommErrorA");
    assert_eq!(
        faults_a[0].typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true)
    );

    // Subsystem B: same fault ID, but should have default (unfailed) state.
    let faults_b = harness.manager.get_all_faults("subsystem_b").unwrap();
    assert_eq!(faults_b.len(), 1);
    assert_eq!(faults_b[0].fault_name, "CommErrorB");
    assert_eq!(
        faults_b[0].typed_status.as_ref().unwrap().confirmed_dtc,
        Some(false),
        "subsystem_b's comm_error should NOT be affected by subsystem_a"
    );
}

/// **Scenario**: Delete faults in one catalog does not affect another.
#[test]
#[serial]
fn delete_faults_scoped_to_catalog() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config(), ivi_catalog_config()]);
    harness.clean_catalogs(&["hvac", "ivi"]);

    // Report faults in both catalogs.
    let hvac_path = make_path("hvac");
    let ivi_path = make_path("ivi");

    harness.processor.process_record(
        &hvac_path,
        &make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed),
    );
    harness.processor.process_record(
        &ivi_path,
        &make_fault_record(
            FaultId::Text(to_static_short_string("ivi.display.init_timeout").unwrap()),
            LifecycleStage::Failed,
        ),
    );

    // Delete all HVAC faults.
    harness.manager.delete_all_faults("hvac").unwrap();

    // HVAC faults are cleared.
    let hvac_faults = harness.manager.get_all_faults("hvac").unwrap();
    let cabin_temp = hvac_faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert_eq!(
        cabin_temp.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(false)
    );

    // IVI fault is unaffected.
    let ivi_faults = harness.manager.get_all_faults("ivi").unwrap();
    assert_eq!(
        ivi_faults[0].typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true),
        "IVI faults should be unaffected by HVAC delete"
    );
}
