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
//! E2E test for JSON catalog loading.
//!
//! Verifies that `FaultCatalogBuilder::json_file()` and `json_string()`
//! correctly load catalog configurations and produce a working catalog
//! that can be used in the DFM pipeline.

use std::io::Write;

use common::{
    catalog::{FaultCatalogBuilder, FaultCatalogConfig},
    fault::*,
};
use serial_test::serial;

use crate::helpers::*;

// ============================================================================
// JSON catalog fixture
// ============================================================================

/// Returns a valid JSON string representing a catalog config.
fn sample_json_catalog() -> &'static str {
    r#"{
        "id": "json_catalog",
        "version": 2,
        "faults": [
            {
                "id": { "Numeric": 1001 },
                "name": "JsonFaultA",
                "summary": "First fault loaded from JSON",
                "category": "Software",
                "severity": "Warn",
                "compliance": [],
                "reporter_side_debounce": null,
                "reporter_side_reset": null,
                "manager_side_debounce": null,
                "manager_side_reset": null
            },
            {
                "id": { "Numeric": 1002 },
                "name": "JsonFaultB",
                "summary": null,
                "category": "Communication",
                "severity": "Error",
                "compliance": ["EmissionRelevant"],
                "reporter_side_debounce": null,
                "reporter_side_reset": null,
                "manager_side_debounce": null,
                "manager_side_reset": null
            }
        ]
    }"#
}

// ============================================================================
// 1. JSON string → catalog → DFM pipeline
// ============================================================================

/// Load a catalog from a JSON string and verify it works end-to-end.
#[test]
#[serial]
fn json_string_catalog_e2e() {
    let catalog = FaultCatalogBuilder::new()
        .json_string(sample_json_catalog())
        .expect("json_string should accept valid JSON")
        .build();

    assert_eq!(catalog.id.as_ref(), "json_catalog");
    assert_eq!(catalog.len(), 2);
    assert!(catalog.descriptor(&FaultId::Numeric(1001)).is_some());
    assert!(catalog.descriptor(&FaultId::Numeric(1002)).is_some());

    // Build config to also test via TestHarness
    let config: FaultCatalogConfig = serde_json::from_str(sample_json_catalog()).unwrap();
    let mut harness = TestHarness::new(vec![config]);
    harness.clean_catalogs(&["json_catalog"]);

    let path = make_path("json_catalog");
    let record = make_fault_record(FaultId::Numeric(1001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    let faults = harness.manager.get_all_faults("json_catalog").unwrap();
    assert_eq!(faults.len(), 2, "JSON catalog should have 2 faults");

    let fault_a = faults.iter().find(|f| f.code == "0x3E9").unwrap(); // 1001 = 0x3E9
    assert_eq!(
        fault_a.typed_status.as_ref().unwrap().test_failed,
        Some(true)
    );
    assert_eq!(fault_a.fault_name, "JsonFaultA");
    assert_eq!(
        fault_a.symptom.as_deref(),
        Some("First fault loaded from JSON")
    );
}

// ============================================================================
// 2. JSON file → catalog → DFM pipeline
// ============================================================================

/// Load a catalog from a JSON file and verify it works end-to-end.
#[test]
#[serial]
fn json_file_catalog_e2e() {
    // Write JSON to a temporary file
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    tmpfile.write_all(sample_json_catalog().as_bytes()).unwrap();
    tmpfile.flush().unwrap();

    let catalog = FaultCatalogBuilder::new()
        .json_file(tmpfile.path().to_path_buf())
        .expect("json_file should accept valid path")
        .build();

    assert_eq!(catalog.id.as_ref(), "json_catalog");
    assert_eq!(catalog.len(), 2);
    assert!(catalog.descriptor(&FaultId::Numeric(1001)).is_some());
    assert!(catalog.descriptor(&FaultId::Numeric(1002)).is_some());

    // Verify descriptor details
    let desc = catalog.descriptor(&FaultId::Numeric(1002)).unwrap();
    assert_eq!(desc.name.to_string(), "JsonFaultB");
    assert_eq!(desc.category, FaultType::Communication);
    assert_eq!(desc.severity, FaultSeverity::Error);
    assert!(
        desc.compliance
            .iter()
            .any(|c| matches!(c, ComplianceTag::EmissionRelevant))
    );
}

// ============================================================================
// 3. JSON string with all fields populated
// ============================================================================

/// JSON with debounce and reset policies deserializes correctly.
#[test]
fn json_string_with_debounce_and_reset() {
    let json = r#"{
        "id": "full_json",
        "version": 5,
        "faults": [
            {
                "id": { "Numeric": 2001 },
                "name": "FullFault",
                "summary": "Full featured fault",
                "category": "Hardware",
                "severity": "Fatal",
                "compliance": ["SafetyCritical", "EmissionRelevant"],
                "reporter_side_debounce": { "HoldTime": { "duration": { "secs": 30, "nanos": 0 } } },
                "reporter_side_reset": null,
                "manager_side_debounce": null,
                "manager_side_reset": {
                    "trigger": { "PowerCycles": 3 },
                    "min_operating_cycles_before_clear": null
                }
            }
        ]
    }"#;

    let catalog = FaultCatalogBuilder::new()
        .json_string(json)
        .unwrap()
        .build();

    assert_eq!(catalog.id.as_ref(), "full_json");
    assert_eq!(catalog.len(), 1);

    let desc = catalog.descriptor(&FaultId::Numeric(2001)).unwrap();
    assert_eq!(desc.severity, FaultSeverity::Fatal);
    assert!(desc.reporter_side_debounce.is_some());
    assert!(desc.manager_side_reset.is_some());
}

// ============================================================================
// 4. Empty JSON faults array
// ============================================================================

/// JSON with empty faults array produces an empty catalog.
#[test]
fn json_string_empty_faults() {
    let json = r#"{ "id": "empty_json", "version": 1, "faults": [] }"#;

    let catalog = FaultCatalogBuilder::new()
        .json_string(json)
        .unwrap()
        .build();

    assert_eq!(catalog.id.as_ref(), "empty_json");
    assert!(catalog.is_empty());
    assert_eq!(catalog.len(), 0);
}
