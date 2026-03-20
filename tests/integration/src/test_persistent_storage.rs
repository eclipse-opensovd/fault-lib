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
//! Persistent storage tests.
//!
//! Verifies that fault state survives across DFM restarts by using the
//! real `KvsSovdFaultStateStorage` (backed by JSON files on disk).
//! This is analogous to persistency module's CIT flush/reload tests.
//!
//! NOTE: All tests use the shared KVS storage directory (process-wide KVS pool
//! constraint) and must be annotated with `#[serial]`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use common::fault::*;
use serial_test::serial;

use crate::helpers::*;

/// **Scenario**: Fault state persists across DFM restart (storage reload).
///
/// Steps:
/// 1. First DFM instance processes a Failed fault → flush to disk
/// 2. Drop the first instance (simulates DFM shutdown)
/// 3. Create a second DFM instance pointing to the same storage dir
/// 4. Query via SOVD — the fault state should still be present
///
/// This mirrors the real deployment where DFM crashes or ECU reboots,
/// and fault history must survive.
///
/// Because the KVS global pool retains instance 0 for the process lifetime,
/// the second harness transparently picks up the same KVS data that was
/// flushed to disk by the first harness.
#[test]
#[serial]
fn fault_state_survives_dfm_restart() {
    // --- First DFM lifetime ---
    {
        let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
        harness.clean_catalogs(&["hvac"]);
        let path = make_path("hvac");

        let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
        harness.processor.process_record(&path, &record);

        // Verify it's stored.
        let faults = harness.manager.get_all_faults("hvac").unwrap();
        let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
        assert_eq!(
            fault.typed_status.as_ref().unwrap().confirmed_dtc,
            Some(true)
        );

        // harness dropped here — DFM shuts down.
    }

    // --- Second DFM lifetime (restart) ---
    {
        let harness = TestHarness::new(vec![hvac_catalog_config()]);

        // Query — the fault state should be recovered from the shared KVS.
        let faults = harness.manager.get_all_faults("hvac").unwrap();
        let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
        let status = fault.typed_status.as_ref().unwrap();

        assert_eq!(
            status.test_failed,
            Some(true),
            "test_failed should persist across restart"
        );
        assert_eq!(
            status.confirmed_dtc,
            Some(true),
            "confirmed_dtc should persist across restart"
        );
    }
}

/// **Scenario**: Delete individual fault clears it from persistent storage.
///
/// After clearing a specific fault via the SOVD delete API, subsequent
/// queries should return default (cleared) status for that fault.
#[test]
#[serial]
fn delete_fault_clears_persistent_state() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // Report fault.
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    // Confirm fault is stored.
    let (fault, _) = harness.manager.get_fault("hvac", "0x7001").unwrap();
    assert_eq!(
        fault.typed_status.as_ref().unwrap().confirmed_dtc,
        Some(true)
    );

    // Delete the fault.
    harness.manager.delete_fault("hvac", "0x7001").unwrap();

    // Query again — should return default status (all flags cleared).
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    let status = fault.typed_status.as_ref().unwrap();
    assert_eq!(
        status.test_failed,
        Some(false),
        "Deleted fault should have default status"
    );
    assert_eq!(status.confirmed_dtc, Some(false));
}

/// **Scenario**: Delete all faults clears the entire catalog's persistent state.
#[test]
#[serial]
fn delete_all_faults_clears_persistent_state() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    let path = make_path("hvac");

    // Report both HVAC faults.
    let record1 = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    let record2 = make_fault_record(
        FaultId::Text(
            common::types::to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap(),
        ),
        LifecycleStage::Failed,
    );
    harness.processor.process_record(&path, &record1);
    harness.processor.process_record(&path, &record2);

    // Verify both are stored.
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    let all_confirmed = faults
        .iter()
        .all(|f| f.typed_status.as_ref().unwrap().confirmed_dtc == Some(true));
    assert!(all_confirmed, "Both faults should be confirmed");

    // Delete all.
    harness.manager.delete_all_faults("hvac").unwrap();

    // Query — all faults should have default (cleared) status.
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    for fault in &faults {
        let status = fault.typed_status.as_ref().unwrap();
        assert_eq!(
            status.test_failed,
            Some(false),
            "Fault {} should be cleared",
            fault.code
        );
        assert_eq!(
            status.confirmed_dtc,
            Some(false),
            "Fault {} should be cleared",
            fault.code
        );
    }
}
