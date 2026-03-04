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
//! Shared helpers for integration tests.
//!
//! Provides a convenience [`TestHarness`] that wires up all DFM components
//! (catalog, registry, processor, storage, SOVD manager) in a single call,
//! matching the real deployment topology minus IPC transport.

use common::catalog::{FaultCatalogBuilder, FaultCatalogConfig};
use common::debounce::DebounceMode;
use common::fault::*;
use common::types::*;
use dfm_lib::fault_catalog_registry::FaultCatalogRegistry;
use dfm_lib::fault_record_processor::FaultRecordProcessor;
use dfm_lib::operation_cycle::OperationCycleTracker;
use dfm_lib::sovd_fault_manager::SovdFaultManager;
use dfm_lib::sovd_fault_storage::KvsSovdFaultStateStorage;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;
use tempfile::TempDir;

/// Shared KVS storage directory used by **all** integration tests.
///
/// KVS uses a process-wide global pool (`KVS_MAX_INSTANCES = 10`) that
/// binds each instance ID to a specific backend path on first use.
/// Subsequent calls with the same instance ID but a *different* path
/// return `InstanceParametersMismatch`.
///
/// To work around this, every test uses the **same** directory for KVS
/// instance 0. Tests are serialized with `#[serial]` to prevent parallel
/// data corruption, and each test cleans the data via `delete_all_faults`.
static SHARED_STORAGE_DIR: LazyLock<TempDir> =
    LazyLock::new(|| TempDir::new().expect("failed to create shared storage dir"));

/// Returns the path of the shared KVS storage directory.
pub fn shared_storage_path() -> &'static Path {
    SHARED_STORAGE_DIR.path()
}

/// All-in-one test harness wiring DFM components together.
///
/// All instances share the same KVS backend directory (process-wide
/// constraint). Tests must run serially (`#[serial]`).
pub struct TestHarness {
    pub processor: FaultRecordProcessor<KvsSovdFaultStateStorage>,
    pub manager: SovdFaultManager<KvsSovdFaultStateStorage>,
}

impl TestHarness {
    /// Build a harness from one or more [`FaultCatalogConfig`]s.
    ///
    /// Each config represents a separate reporter application's fault catalog
    /// (e.g., HVAC, IVI), mirroring how multiple apps register with a single DFM.
    pub fn new(configs: Vec<FaultCatalogConfig>) -> Self {
        Self::with_storage_path(configs, shared_storage_path())
    }

    /// Build a harness using an explicit storage path. Useful for persistence
    /// tests where the same directory is reused across harness instances.
    pub fn with_storage_path(configs: Vec<FaultCatalogConfig>, storage_path: &Path) -> Self {
        let storage =
            Arc::new(KvsSovdFaultStateStorage::new(storage_path, 0).expect("storage init"));
        let catalogs: Vec<_> = configs
            .into_iter()
            .map(|cfg| {
                FaultCatalogBuilder::new()
                    .cfg_struct(cfg)
                    .expect("builder config")
                    .build()
            })
            .collect();
        let registry = Arc::new(FaultCatalogRegistry::new(catalogs));
        let cycle_tracker = Arc::new(RwLock::new(OperationCycleTracker::new()));

        let processor =
            FaultRecordProcessor::new(Arc::clone(&storage), Arc::clone(&registry), cycle_tracker);
        let manager = SovdFaultManager::new(storage, registry);

        Self { processor, manager }
    }

    /// Clean all fault data from the shared storage.
    ///
    /// Call this at the start of each test to ensure a clean slate
    /// (defence-in-depth alongside `#[serial]`).
    pub fn clean_catalogs(&mut self, paths: &[&str]) {
        for path in paths {
            // Ignore errors — the path may not have data yet.
            let _ = self.manager.delete_all_faults(path);
        }
    }
}

// ============================================================================
// Catalog configs
// ============================================================================

/// HVAC subsystem catalog with two faults:
/// - `CabinTempSensorStuck` (Numeric 0x7001, reporter-side HoldTime debounce)
/// - `BlowerSpeedMismatch` (Text, manager-side EdgeWithCooldown debounce)
pub fn hvac_catalog_config() -> FaultCatalogConfig {
    FaultCatalogConfig {
        id: "hvac".into(),
        version: 3,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Numeric(0x7001),
                name: to_static_short_string("CabinTempSensorStuck").unwrap(),
                summary: None,
                category: FaultType::Communication,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::try_from(&[ComplianceTag::EmissionRelevant][..])
                    .unwrap(),
                reporter_side_debounce: Some(DebounceMode::HoldTime {
                    duration: Duration::from_secs(60).into(),
                }),
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Text(
                    to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap(),
                ),
                name: to_static_short_string("BlowerSpeedMismatch").unwrap(),
                summary: Some(
                    to_static_long_string("Blower motor speed does not match commanded value")
                        .unwrap(),
                ),
                category: FaultType::Communication,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::try_from(
                    &[
                        ComplianceTag::SecurityRelevant,
                        ComplianceTag::SafetyCritical,
                    ][..],
                )
                .unwrap(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(100).into(),
                }),
                manager_side_reset: None,
            },
        ],
    }
}

/// IVI (In-Vehicle Infotainment) catalog with a single software fault.
pub fn ivi_catalog_config() -> FaultCatalogConfig {
    FaultCatalogConfig {
        id: "ivi".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Text(to_static_short_string("ivi.display.init_timeout").unwrap()),
            name: to_static_short_string("DisplayInitTimeout").unwrap(),
            summary: Some(
                to_static_long_string("Display initialization exceeded 5s timeout").unwrap(),
            ),
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    }
}

// ============================================================================
// Record builders
// ============================================================================

/// Build a [`FaultRecord`] simulating what a reporter would create.
pub fn make_fault_record(fault_id: FaultId, stage: LifecycleStage) -> FaultRecord {
    FaultRecord {
        id: fault_id,
        time: IpcTimestamp::default(),
        source: common::SourceId {
            entity: to_static_short_string("test_reporter").unwrap(),
            ecu: Some(to_static_short_string("ECU-A").unwrap()),
            domain: Some(to_static_short_string("body").unwrap()),
            sw_component: Some(to_static_short_string("hvac_ctrl").unwrap()),
            instance: Some(to_static_short_string("0").unwrap()),
        },
        lifecycle_phase: LifecyclePhase::Running,
        lifecycle_stage: stage,
        env_data: MetadataVec::new(),
    }
}

/// Build a [`FaultRecord`] with environment data attached.
pub fn make_fault_record_with_env(
    fault_id: FaultId,
    stage: LifecycleStage,
    env: &[(&str, &str)],
) -> FaultRecord {
    let env_data = MetadataVec::try_from(
        &env.iter()
            .map(|(k, v)| {
                (
                    to_static_short_string(k).unwrap(),
                    to_static_short_string(v).unwrap(),
                )
            })
            .collect::<Vec<_>>()[..],
    )
    .unwrap();

    FaultRecord {
        id: fault_id,
        time: IpcTimestamp::default(),
        source: common::SourceId {
            entity: to_static_short_string("test_reporter").unwrap(),
            ecu: Some(to_static_short_string("ECU-A").unwrap()),
            domain: Some(to_static_short_string("body").unwrap()),
            sw_component: Some(to_static_short_string("hvac_ctrl").unwrap()),
            instance: Some(to_static_short_string("0").unwrap()),
        },
        lifecycle_phase: LifecyclePhase::Running,
        lifecycle_stage: stage,
        env_data,
    }
}

/// Helper to create a [`LongString`] path for DFM routing.
pub fn make_path(path: &str) -> LongString {
    LongString::from_str_truncated(path).unwrap()
}
