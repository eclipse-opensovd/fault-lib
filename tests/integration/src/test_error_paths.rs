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
//! Error-path integration tests.
//!
//! These tests exercise fault-lib error handling and validation at the
//! integration level, complementing the happy-path tests in other modules.
//!
//! Scenarios covered:
//! - Processing records with unknown fault IDs
//! - Empty catalog behavior
//! - KVS storage initialization failures
//! - Catalog builder double-configuration detection
//! - Duplicate `FaultId` detection in catalog configs

use common::{
    catalog::{CatalogBuildError, FaultCatalogBuilder, FaultCatalogConfig},
    fault::*,
    types::to_static_short_string,
};
use dfm_lib::sovd_fault_storage::KvsSovdFaultStateStorage;
use serial_test::serial;

use crate::helpers::*;

// ============================================================================
// 1. Invalid Fault ID
// ============================================================================

/// Processing a record with a fault ID not in any registered catalog
/// must not panic. The processor handles unrecognised faults gracefully
/// and existing catalog faults remain queryable.
#[test]
#[serial]
fn process_record_with_unknown_fault_id_does_not_panic() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let unknown_id = FaultId::Numeric(0xDEAD);
    let record = make_fault_record(unknown_id, LifecycleStage::Failed);

    // Must not panic or corrupt state
    harness.processor.process_record(&path, &record);

    // Known catalog faults remain intact
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(
        faults.len(),
        2,
        "Known catalog faults should still be returned"
    );
}

/// An unknown fault ID processed multiple times does not accumulate
/// unexpected state or cause the processor to misbehave.
#[test]
#[serial]
fn repeated_unknown_fault_id_does_not_corrupt_state() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let unknown_id = FaultId::Text(to_static_short_string("nonexistent.sensor").unwrap());

    // Process several records with the unknown ID
    for stage in [
        LifecycleStage::Failed,
        LifecycleStage::Passed,
        LifecycleStage::Failed,
    ] {
        let record = make_fault_record(unknown_id.clone(), stage);
        harness.processor.process_record(&path, &record);
    }

    // Catalog faults are unaffected
    let faults = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(faults.len(), 2);
}

// ============================================================================
// 2. Empty Catalog
// ============================================================================

/// An empty fault catalog (zero descriptors) is valid but yields no faults.
#[test]
#[serial]
fn empty_catalog_query_returns_no_faults() {
    let empty_config = FaultCatalogConfig {
        id: "empty_test".into(),
        version: 1,
        faults: vec![],
    };
    let mut harness = TestHarness::new(vec![empty_config]);
    harness.clean_catalogs(&["empty_test"]);

    let faults = harness.manager.get_all_faults("empty_test").unwrap();
    assert!(
        faults.is_empty(),
        "Empty catalog should return no faults, got: {}",
        faults.len()
    );
}

/// Processing a record against an empty catalog does not panic.
#[test]
#[serial]
fn process_record_against_empty_catalog_does_not_panic() {
    let empty_config = FaultCatalogConfig {
        id: "empty_process".into(),
        version: 1,
        faults: vec![],
    };
    let mut harness = TestHarness::new(vec![empty_config]);
    harness.clean_catalogs(&["empty_process"]);

    let path = make_path("empty_process");
    let record = make_fault_record(FaultId::Numeric(42), LifecycleStage::Failed);

    // Must not panic even though the fault ID is not in the empty catalog
    harness.processor.process_record(&path, &record);
}

// ============================================================================
// 3. Corrupt KVS
// ============================================================================

/// KVS storage rejects re-initialisation of an already-bound instance ID
/// with a different path, returning a `StorageError`.
///
/// The process-wide KVS pool binds each instance ID to a specific backend
/// path. Attempting to reuse an instance with a different path must fail.
#[test]
#[serial]
fn kvs_rejects_instance_id_reuse_with_different_path() {
    // Ensure instance 0 is bound to the shared storage path
    let _harness = TestHarness::new(vec![hvac_catalog_config()]);

    // Try to rebind instance 0 to a completely different path
    let other_dir = tempfile::TempDir::new().unwrap();
    let result = KvsSovdFaultStateStorage::new(other_dir.path(), 0);
    assert!(
        result.is_err(),
        "KVS should reject reuse of instance 0 with a different path"
    );
}

/// KVS storage should fail when given a regular file instead of a directory.
#[test]
fn kvs_storage_rejects_file_as_storage_path() {
    let tmpfile = tempfile::NamedTempFile::new().unwrap();
    // tmpfile.path() points to a regular file, not a directory
    let result = KvsSovdFaultStateStorage::new(tmpfile.path(), 7);
    assert!(
        result.is_err(),
        "KVS should reject a regular file as storage path"
    );
}

// ============================================================================
// 4. Double Init (Catalog Builder)
// ============================================================================

/// The builder rejects a second `cfg_struct()` call with
/// `AlreadyConfigured`.
#[test]
fn catalog_builder_rejects_double_configure() {
    let config = FaultCatalogConfig {
        id: "double_init".into(),
        version: 1,
        faults: vec![],
    };

    let result = FaultCatalogBuilder::new()
        .cfg_struct(config.clone())
        .expect("first cfg_struct should succeed")
        .cfg_struct(config);

    let err = result.err().expect("second cfg_struct should fail");
    assert!(
        matches!(err, CatalogBuildError::AlreadyConfigured),
        "Second cfg_struct() should return AlreadyConfigured, got: {err:?}"
    );
}

/// Building without any configuration returns `MissingConfig`.
#[test]
fn catalog_builder_rejects_no_config() {
    let result = FaultCatalogBuilder::new().try_build();
    assert!(
        matches!(result, Err(CatalogBuildError::MissingConfig)),
        "Building with no config should return MissingConfig, got: {result:?}"
    );
}

/// Invalid JSON string triggers `InvalidJson`.
#[test]
fn catalog_builder_rejects_invalid_json() {
    let result = FaultCatalogBuilder::new()
        .json_string("{ not valid json }")
        .expect("json_string accepts any string")
        .try_build();

    assert!(
        matches!(result, Err(CatalogBuildError::InvalidJson(_))),
        "Invalid JSON should return InvalidJson, got: {result:?}"
    );
}

/// Non-existent JSON file path triggers `Io` error.
#[test]
fn catalog_builder_rejects_missing_json_file() {
    let result = FaultCatalogBuilder::new()
        .json_file(std::path::PathBuf::from("/nonexistent/path/catalog.json"))
        .expect("json_file accepts any path")
        .try_build();

    assert!(
        matches!(result, Err(CatalogBuildError::Io(_))),
        "Missing file should return Io error, got: {result:?}"
    );
}

// ============================================================================
// 5. Duplicate FaultId
// ============================================================================

/// A catalog config containing two faults with the same numeric ID must
/// be rejected with `DuplicateFaultId`.
#[test]
fn catalog_builder_rejects_duplicate_numeric_fault_id() {
    let dup_config = FaultCatalogConfig {
        id: "dup_numeric".into(),
        version: 1,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Numeric(0x1234),
                name: to_static_short_string("Fault A").unwrap(),
                summary: None,
                category: FaultType::Software,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Numeric(0x1234), // Duplicate!
                name: to_static_short_string("Fault B").unwrap(),
                summary: None,
                category: FaultType::Communication,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
        ],
    };

    let result = FaultCatalogBuilder::new()
        .cfg_struct(dup_config)
        .expect("cfg_struct accepts any config")
        .try_build();

    assert!(
        matches!(result, Err(CatalogBuildError::DuplicateFaultId(_))),
        "Should return DuplicateFaultId, got: {result:?}"
    );
}

/// Duplicate text-based `FaultIds` are also caught.
#[test]
fn catalog_builder_rejects_duplicate_text_fault_id() {
    let dup_config = FaultCatalogConfig {
        id: "dup_text".into(),
        version: 1,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("sensor.temp.stuck").unwrap()),
                name: to_static_short_string("TempStuck A").unwrap(),
                summary: None,
                category: FaultType::Software,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("sensor.temp.stuck").unwrap()),
                name: to_static_short_string("TempStuck B").unwrap(),
                summary: None,
                category: FaultType::Communication,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
        ],
    };

    let result = FaultCatalogBuilder::new()
        .cfg_struct(dup_config)
        .expect("cfg_struct accepts any config")
        .try_build();

    assert!(
        matches!(result, Err(CatalogBuildError::DuplicateFaultId(_))),
        "Should return DuplicateFaultId, got: {result:?}"
    );
}
