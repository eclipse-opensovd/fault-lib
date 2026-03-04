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
//! Boundary value integration tests.
//!
//! Tests on extreme/edge values: max-length fault IDs, max faults in a
//! catalog, many catalogs, env data limits, and Unicode in identifiers.

use crate::helpers::*;
use common::catalog::FaultCatalogConfig;
use common::fault::*;
use common::types::*;
use serial_test::serial;

// ============================================================================
// 1. Max-length fault IDs
// ============================================================================

/// A text FaultId at the maximum ShortString length (64 bytes) should
/// work correctly through the full pipeline.
#[test]
#[serial]
fn max_length_text_fault_id() {
    // ShortString is 64 bytes; use a string that fills it exactly
    let max_id = "a".repeat(63); // 63 chars + null = 64 bytes in StaticString
    let fault_id = FaultId::Text(to_static_short_string(&max_id).unwrap());

    let config = FaultCatalogConfig {
        id: "boundary_text".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("MaxLenFault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["boundary_text"]);

    let path = make_path("boundary_text");
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("boundary_text").unwrap();
    assert_eq!(faults.len(), 1);
    let fault = &faults[0];
    assert_eq!(fault.code, max_id);
    assert_eq!(fault.typed_status.as_ref().unwrap().test_failed, Some(true));
}

/// UUID FaultId (all zeros) boundary value works through pipeline.
#[test]
#[serial]
fn uuid_fault_id_all_zeros() {
    let fault_id = FaultId::Uuid([0u8; 16]);

    let config = FaultCatalogConfig {
        id: "boundary_uuid".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("ZeroUuid").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["boundary_uuid"]);

    let path = make_path("boundary_uuid");
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("boundary_uuid").unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].code, "00000000-0000-0000-0000-000000000000");
}

/// UUID FaultId (all 0xFF) boundary value works through pipeline.
#[test]
#[serial]
fn uuid_fault_id_all_max() {
    let fault_id = FaultId::Uuid([0xFF; 16]);

    let config = FaultCatalogConfig {
        id: "boundary_uuid_max".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("MaxUuid").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["boundary_uuid_max"]);

    let path = make_path("boundary_uuid_max");
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("boundary_uuid_max").unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].code, "ffffffff-ffff-ffff-ffff-ffffffffffff");
}

// ============================================================================
// 2. Max faults in a catalog
// ============================================================================

/// A catalog with many faults (50) builds and queries correctly.
#[test]
#[serial]
fn catalog_with_many_faults() {
    let faults: Vec<FaultDescriptor> = (0..50u32)
        .map(|i| FaultDescriptor {
            id: FaultId::Numeric(i),
            name: to_static_short_string(format!("Fault_{i}")).unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        })
        .collect();

    let config = FaultCatalogConfig {
        id: "many_faults".into(),
        version: 1,
        faults,
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["many_faults"]);

    // Process a record for every fault
    let path = make_path("many_faults");
    for i in 0..50u32 {
        let record = make_fault_record(FaultId::Numeric(i), LifecycleStage::Failed);
        harness.processor.process_record(&path, &record);
    }

    let faults = harness.manager.get_all_faults("many_faults").unwrap();
    assert_eq!(faults.len(), 50, "All 50 faults should be queryable");

    // Verify all are marked as failed
    for fault in &faults {
        assert_eq!(
            fault.typed_status.as_ref().unwrap().test_failed,
            Some(true),
            "Fault {} should be marked as failed",
            fault.code
        );
    }
}

// ============================================================================
// 3. Max catalogs
// ============================================================================

/// Multiple catalogs (5) registered simultaneously work correctly.
#[test]
#[serial]
fn many_catalogs_registered() {
    let configs: Vec<FaultCatalogConfig> = (0..5)
        .map(|i| FaultCatalogConfig {
            id: format!("catalog_{i}").into(),
            version: 1,
            faults: vec![FaultDescriptor {
                id: FaultId::Numeric(i * 100),
                name: to_static_short_string(format!("Fault_C{i}")).unwrap(),
                summary: None,
                category: FaultType::Software,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            }],
        })
        .collect();

    let catalog_ids: Vec<String> = (0..5).map(|i| format!("catalog_{i}")).collect();
    let catalog_refs: Vec<&str> = catalog_ids
        .iter()
        .map(std::string::String::as_str)
        .collect();

    let mut harness = TestHarness::new(configs);
    harness.clean_catalogs(&catalog_refs);

    // Process one fault per catalog
    for i in 0..5u32 {
        let path = make_path(&format!("catalog_{i}"));
        let record = make_fault_record(FaultId::Numeric(i * 100), LifecycleStage::Failed);
        harness.processor.process_record(&path, &record);
    }

    // Each catalog should have exactly 1 fault
    for i in 0..5 {
        let faults = harness
            .manager
            .get_all_faults(&format!("catalog_{i}"))
            .unwrap();
        assert_eq!(faults.len(), 1, "catalog_{i} should have 1 fault");
        assert_eq!(
            faults[0].typed_status.as_ref().unwrap().test_failed,
            Some(true)
        );
    }
}

// ============================================================================
// 4. Env data limits
// ============================================================================

/// Fault with maximum env data entries (MetadataVec capacity = 8) works.
#[test]
#[serial]
fn fault_with_max_env_data_entries() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let env: Vec<(&str, &str)> = vec![
        ("key0", "val0"),
        ("key1", "val1"),
        ("key2", "val2"),
        ("key3", "val3"),
        ("key4", "val4"),
        ("key5", "val5"),
        ("key6", "val6"),
        ("key7", "val7"),
    ];
    let record = make_fault_record_with_env(FaultId::Numeric(0x7001), LifecycleStage::Failed, &env);
    harness.processor.process_record(&path, &record);

    let (fault, env_data) = harness.manager.get_fault("hvac", "0x7001").unwrap();
    assert_eq!(fault.typed_status.as_ref().unwrap().test_failed, Some(true));
    assert_eq!(
        env_data.len(),
        8,
        "All 8 env data entries should be preserved"
    );
    assert_eq!(env_data.get("key0"), Some(&"val0".to_string()));
    assert_eq!(env_data.get("key7"), Some(&"val7".to_string()));
}

// ============================================================================
// 5. Unicode in fault IDs and env data
// ============================================================================

/// Non-ASCII (Unicode) characters in text FaultIds are rejected by
/// `StaticString` which only accepts ASCII. This is a known API boundary.
#[test]
fn unicode_in_text_fault_id_rejected() {
    let result = to_static_short_string("sensor.温度.stuck");
    assert!(
        result.is_err(),
        "StaticString should reject non-ASCII characters"
    );
}

/// ASCII-only text FaultIds with special characters work correctly.
#[test]
#[serial]
fn special_ascii_chars_in_text_fault_id() {
    let special_id = "sensor.temp-123_v2.stuck";
    let fault_id = FaultId::Text(to_static_short_string(special_id).unwrap());

    let config = FaultCatalogConfig {
        id: "ascii_special".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("SpecialAscii").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["ascii_special"]);

    let path = make_path("ascii_special");
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("ascii_special").unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].code, special_id);
    assert_eq!(
        faults[0].typed_status.as_ref().unwrap().test_failed,
        Some(true)
    );
}

/// Non-ASCII (Unicode) env data values are rejected by StaticString.
/// This documents the API boundary for IPC-safe types.
#[test]
fn unicode_in_env_data_rejected() {
    let result = to_static_short_string("エンジンルーム");
    assert!(
        result.is_err(),
        "StaticString should reject non-ASCII env data values"
    );
}

/// ASCII env data with special characters works correctly.
#[test]
#[serial]
fn special_ascii_in_env_data() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let env = [("location", "engine-room_v2"), ("status", "fault:active")];
    let record = make_fault_record_with_env(FaultId::Numeric(0x7001), LifecycleStage::Failed, &env);
    harness.processor.process_record(&path, &record);

    let (_, env_data) = harness.manager.get_fault("hvac", "0x7001").unwrap();
    assert_eq!(
        env_data.get("location"),
        Some(&"engine-room_v2".to_string())
    );
    assert_eq!(env_data.get("status"), Some(&"fault:active".to_string()));
}

/// Numeric FaultId boundary: u32::MAX works through the pipeline.
#[test]
#[serial]
fn numeric_fault_id_max_u32() {
    let fault_id = FaultId::Numeric(u32::MAX);

    let config = FaultCatalogConfig {
        id: "boundary_u32".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("MaxNumeric").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };

    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["boundary_u32"]);

    let path = make_path("boundary_u32");
    let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("boundary_u32").unwrap();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].code, "0xFFFFFFFF");
    assert_eq!(
        faults[0].typed_status.as_ref().unwrap().test_failed,
        Some(true)
    );
}
