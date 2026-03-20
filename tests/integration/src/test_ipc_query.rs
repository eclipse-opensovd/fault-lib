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

//! Integration tests for the DFM query/clear API.
//!
//! **Part 1 - `DirectDfmQuery` baseline:** process faults via `FaultRecordProcessor`,
//! then query/clear via `SovdFaultManager`. Covers `get_all`, `get_fault`, `delete_single`,
//! `delete_all`, `bad_path`, `not_found`.
//!
//! **Part 2 - IPC E2E:** `DiagnosticFaultManager::with_query_server()` + `Iceoryx2DfmQuery`
//! over real iceoryx2 shared-memory transport.

use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use common::{
    catalog::FaultCatalogBuilder,
    fault::{FaultId, LifecycleStage},
};
use dfm_lib::{
    diagnostic_fault_manager::DiagnosticFaultManager, fault_catalog_registry::FaultCatalogRegistry,
    fault_record_processor::FaultRecordProcessor, operation_cycle::OperationCycleTracker,
    query_api::DfmQueryApi, query_ipc::Iceoryx2DfmQuery, sovd_fault_manager::Error,
    sovd_fault_storage::KvsSovdFaultStateStorage,
};
use serial_test::serial;

use crate::helpers::*;

/// Global counter for unique KVS instance IDs.
/// Starts at 2 to avoid conflict with instance 0 (shared tests) and 1.
static KVS_INSTANCE_COUNTER: AtomicUsize = AtomicUsize::new(2);

#[test]
#[serial]
fn direct_query_baseline() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    let path = make_path("hvac");
    harness.processor.process_record(&path, &record);

    let query = harness.manager.get_all_faults("hvac").unwrap();
    assert_eq!(query.len(), 2); // 2 descriptors in hvac catalog

    let fault = query.iter().find(|f| f.code == "0x7001").unwrap();
    assert!(fault.typed_status.as_ref().unwrap().test_failed.unwrap());
    assert_eq!(fault.occurrence_counter, Some(1));
}

#[test]
#[serial]
fn direct_query_get_fault_with_env_data() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let record = make_fault_record_with_env(
        FaultId::Numeric(0x7001),
        LifecycleStage::Failed,
        &[("temp", "42"), ("pressure", "1013")],
    );
    let path = make_path("hvac");
    harness.processor.process_record(&path, &record);

    let (fault, env) = harness.manager.get_fault("hvac", "0x7001").unwrap();
    assert_eq!(fault.code, "0x7001");
    assert_eq!(env.get("temp"), Some(&"42".into()));
}

#[test]
#[serial]
fn direct_query_delete_single_fault() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    let path = make_path("hvac");
    harness.processor.process_record(&path, &record);

    harness.manager.delete_fault("hvac", "0x7001").unwrap();

    // Fault still appears (descriptor exists) but status is cleared
    let (fault, _) = harness.manager.get_fault("hvac", "0x7001").unwrap();
    assert!(!fault.typed_status.as_ref().unwrap().test_failed.unwrap());
}

#[test]
#[serial]
fn direct_query_delete_all_faults() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    let path = make_path("hvac");
    harness.processor.process_record(&path, &record);

    harness.manager.delete_all_faults("hvac").unwrap();

    let faults = harness.manager.get_all_faults("hvac").unwrap();
    // All faults show default/cleared status
    for fault in &faults {
        assert!(!fault.typed_status.as_ref().unwrap().test_failed.unwrap());
    }
}

#[test]
#[serial]
fn direct_query_bad_path_returns_bad_argument() {
    let harness = TestHarness::new(vec![hvac_catalog_config()]);
    assert_eq!(
        harness.manager.get_all_faults("nonexistent"),
        Err(Error::BadArgument)
    );
}

#[test]
#[serial]
fn direct_query_not_found_fault_code() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);
    assert_eq!(
        harness.manager.get_fault("hvac", "0xFFFF"),
        Err(Error::NotFound)
    );
}

// ============================================================================
// Part 2: IPC E2E tests (DiagnosticFaultManager + Iceoryx2DfmQuery)
// ============================================================================

/// Helper: create a pre-populated KVS storage with faults processed,
/// then build a `DiagnosticFaultManager::with_query_server()` on top.
///
/// Returns the DFM (must be kept alive for the server to run).
/// Uses a dedicated temp dir + KVS instance to avoid conflicts.
fn start_dfm_with_faults(
    configs: Vec<common::catalog::FaultCatalogConfig>,
    faults: &[(FaultId, LifecycleStage)],
    entity: &str,
) -> (
    DiagnosticFaultManager<KvsSovdFaultStateStorage>,
    tempfile::TempDir,
) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let instance_id = KVS_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let storage =
        Arc::new(KvsSovdFaultStateStorage::new(dir.path(), instance_id).expect("storage"));
    let catalogs: Vec<_> = configs
        .iter()
        .map(|cfg| {
            FaultCatalogBuilder::new()
                .cfg_struct(cfg.clone())
                .expect("builder")
                .build()
        })
        .collect();
    let registry = Arc::new(FaultCatalogRegistry::new(catalogs));
    let cycle_tracker = Arc::new(RwLock::new(OperationCycleTracker::new()));

    // Process faults into storage, then drop processor + registry + tracker
    // to release Arc references so we can unwrap the storage.
    {
        let mut processor =
            FaultRecordProcessor::new(Arc::clone(&storage), Arc::clone(&registry), cycle_tracker);
        let path = make_path(entity);
        for (id, stage) in faults {
            let record = make_fault_record(id.clone(), *stage);
            processor.process_record(&path, &record);
        }
    }

    // Unwrap Arc to pass owned storage to DiagnosticFaultManager
    drop(registry);
    let storage = Arc::try_unwrap(storage).ok().expect("all Arc refs dropped");

    let dfm_registry = FaultCatalogRegistry::new(
        configs
            .into_iter()
            .map(|cfg| {
                FaultCatalogBuilder::new()
                    .cfg_struct(cfg)
                    .expect("builder")
                    .build()
            })
            .collect(),
    );

    let dfm = DiagnosticFaultManager::with_query_server(storage, dfm_registry);

    // Wait for query server to be available (retry with timeout instead of fixed sleep)
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match Iceoryx2DfmQuery::new() {
            Ok(_) => break,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("DFM query server not ready within 2s: {e}"),
        }
    }
    // Allow DFM worker to process queued fault events after IPC is ready
    std::thread::sleep(Duration::from_millis(50));

    (dfm, dir)
}

#[test]
#[serial(ipc)]
fn ipc_e2e_query_all_faults() {
    let (dfm, _dir) = start_dfm_with_faults(
        vec![hvac_catalog_config()],
        &[(FaultId::Numeric(0x7001), LifecycleStage::Failed)],
        "hvac",
    );

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    let faults = client.get_all_faults("hvac").unwrap();
    assert_eq!(faults.len(), 2); // 2 descriptors in hvac catalog

    let fault = faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert!(fault.typed_status.as_ref().unwrap().test_failed.unwrap());
    assert_eq!(fault.occurrence_counter, Some(1));

    drop(dfm);
}

#[test]
#[serial(ipc)]
fn ipc_e2e_get_single_fault() {
    let (dfm, _dir) = start_dfm_with_faults(
        vec![hvac_catalog_config()],
        &[(FaultId::Numeric(0x7001), LifecycleStage::Failed)],
        "hvac",
    );

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    let (fault, _env) = client.get_fault("hvac", "0x7001").unwrap();
    assert_eq!(fault.code, "0x7001");
    assert!(fault.typed_status.as_ref().unwrap().test_failed.unwrap());

    drop(dfm);
}

#[test]
#[serial(ipc)]
fn ipc_e2e_delete_single_fault() {
    let (dfm, _dir) = start_dfm_with_faults(
        vec![hvac_catalog_config()],
        &[(FaultId::Numeric(0x7001), LifecycleStage::Failed)],
        "hvac",
    );

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    client.delete_fault("hvac", "0x7001").unwrap();

    let (fault, _) = client.get_fault("hvac", "0x7001").unwrap();
    assert!(!fault.typed_status.as_ref().unwrap().test_failed.unwrap());

    drop(dfm);
}

#[test]
#[serial(ipc)]
fn ipc_e2e_delete_all_faults() {
    let (dfm, _dir) = start_dfm_with_faults(
        vec![hvac_catalog_config()],
        &[(FaultId::Numeric(0x7001), LifecycleStage::Failed)],
        "hvac",
    );

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    client.delete_all_faults("hvac").unwrap();

    let faults = client.get_all_faults("hvac").unwrap();
    for fault in &faults {
        assert!(!fault.typed_status.as_ref().unwrap().test_failed.unwrap());
    }

    drop(dfm);
}

#[test]
#[serial(ipc)]
fn ipc_e2e_bad_path_returns_bad_argument() {
    let (dfm, _dir) = start_dfm_with_faults(vec![hvac_catalog_config()], &[], "hvac");

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    assert_eq!(
        client.get_all_faults("nonexistent"),
        Err(Error::BadArgument)
    );

    drop(dfm);
}

#[test]
#[serial(ipc)]
fn ipc_e2e_not_found_fault_code() {
    let (dfm, _dir) = start_dfm_with_faults(vec![hvac_catalog_config()], &[], "hvac");

    let client = Iceoryx2DfmQuery::new().expect("IPC client");
    assert_eq!(client.get_fault("hvac", "0xFFFF"), Err(Error::NotFound));

    drop(dfm);
}
