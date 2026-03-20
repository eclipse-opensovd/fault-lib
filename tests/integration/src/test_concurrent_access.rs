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
//! Concurrent access integration tests.
//!
//! These tests verify thread-safety under contention: concurrent
//! `process_record` + `get_fault`, concurrent delete + process, and
//! multi-catalog concurrent access.

use std::{sync::Arc, thread};

use common::fault::*;
use serial_test::serial;

use crate::helpers::*;

// ============================================================================
// 1. Concurrent process_record + get_fault on the same fault
// ============================================================================

/// Concurrent writes (`process_record`) and reads (`get_all_faults`)
/// on the same catalog must not panic or corrupt data.
///
/// The outer `Mutex<TestHarness>` is architecturally necessary because
/// `FaultRecordProcessor` requires `&mut self`. This test validates
/// that the lock acquisition + operation sequence does not deadlock
/// or panic under thread contention.
#[test]
#[serial]
fn concurrent_process_and_query_does_not_panic() {
    let harness = Arc::new(std::sync::Mutex::new(TestHarness::new(vec![
        hvac_catalog_config(),
    ])));

    // Clean slate
    harness.lock().unwrap().clean_catalogs(&["hvac"]);

    let path = make_path("hvac");
    let fault_id = FaultId::Numeric(0x7001);

    // Pre-populate with a known state
    {
        let mut h = harness.lock().unwrap();
        let record = make_fault_record(fault_id.clone(), LifecycleStage::Failed);
        h.processor.process_record(&path, &record);
    }

    // Spawn concurrent readers and writers
    let mut handles = vec![];

    for _ in 0..5 {
        let harness = Arc::clone(&harness);
        let fault_id = fault_id.clone();
        handles.push(thread::spawn(move || {
            for stage in [
                LifecycleStage::Failed,
                LifecycleStage::Passed,
                LifecycleStage::PreFailed,
            ] {
                let mut h = harness.lock().unwrap();
                let record = make_fault_record(fault_id.clone(), stage);
                h.processor.process_record(&path, &record);
            }
        }));
    }

    for _ in 0..5 {
        let harness = Arc::clone(&harness);
        handles.push(thread::spawn(move || {
            for _ in 0..3 {
                let h = harness.lock().unwrap();
                let result = h.manager.get_all_faults("hvac");
                assert!(
                    result.is_ok(),
                    "get_all_faults should not fail under contention"
                );
                let faults = result.unwrap();
                assert!(!faults.is_empty(), "Should always have faults in catalog");
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    // Final state should be valid
    let h = harness.lock().unwrap();
    let faults = h.manager.get_all_faults("hvac").unwrap();
    assert_eq!(faults.len(), 2, "HVAC catalog has 2 descriptors");
}

// ============================================================================
// 1b. Concurrent reads on SovdFaultManager without outer Mutex
// ============================================================================

/// Concurrent reads on `SovdFaultManager` without an outer Mutex.
///
/// Unlike the test above, this exercises real concurrency: `SovdFaultManager`
/// is internally thread-safe (uses `Arc<S>` where `S: SovdFaultStateStorage`
/// provides its own `Mutex<Kvs>`), so sharing it via `Arc` across threads
/// without an outer lock is safe and tests the actual concurrent code path.
#[test]
#[serial]
fn concurrent_reads_on_manager_does_not_panic() {
    let mut harness = TestHarness::new(vec![hvac_catalog_config()]);
    harness.clean_catalogs(&["hvac"]);

    // Pre-populate with known state (single-threaded setup).
    let path = make_path("hvac");
    let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
    harness.processor.process_record(&path, &record);

    // Share only the manager (internally thread-safe) via Arc.
    let manager = Arc::new(harness.manager);
    let mut handles = vec![];

    for _ in 0..10 {
        let mgr = Arc::clone(&manager);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                let result = mgr.get_all_faults("hvac");
                assert!(
                    result.is_ok(),
                    "get_all_faults should not fail under concurrent reads"
                );
                let faults = result.unwrap();
                assert!(!faults.is_empty(), "Should always have faults in catalog");
            }
        }));
    }

    for h in handles {
        h.join().expect("Reader thread should not panic");
    }
}

// ============================================================================
// 2. Concurrent delete + process on the same fault
// ============================================================================

/// Concurrent `delete_all_faults` and `process_record` must not panic.
/// The storage layer must handle interleaved writes and deletes gracefully.
#[test]
#[serial]
fn concurrent_delete_and_process_does_not_panic() {
    let harness = Arc::new(std::sync::Mutex::new(TestHarness::new(vec![
        hvac_catalog_config(),
    ])));
    harness.lock().unwrap().clean_catalogs(&["hvac"]);

    let path = make_path("hvac");

    // Pre-populate
    {
        let mut h = harness.lock().unwrap();
        let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
        h.processor.process_record(&path, &record);
    }

    let mut handles = vec![];

    // Writers: repeatedly process records
    for _ in 0..3 {
        let harness = Arc::clone(&harness);
        handles.push(thread::spawn(move || {
            for _ in 0..5 {
                let mut h = harness.lock().unwrap();
                let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
                h.processor.process_record(&path, &record);
            }
        }));
    }

    // Deleters: repeatedly delete all faults
    for _ in 0..3 {
        let harness = Arc::clone(&harness);
        handles.push(thread::spawn(move || {
            for _ in 0..5 {
                let h = harness.lock().unwrap();
                let _ = h.manager.delete_all_faults("hvac");
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread should not panic");
    }
}

// ============================================================================
// 3. Multi-catalog concurrent access
// ============================================================================

/// Concurrent access to different catalogs (HVAC and IVI) must maintain
/// isolation — writes to one catalog must not affect the other.
#[test]
#[serial]
fn multi_catalog_concurrent_access_maintains_isolation() {
    let harness = Arc::new(std::sync::Mutex::new(TestHarness::new(vec![
        hvac_catalog_config(),
        ivi_catalog_config(),
    ])));
    harness.lock().unwrap().clean_catalogs(&["hvac", "ivi"]);

    let mut handles = vec![];

    // HVAC writer thread
    {
        let harness = Arc::clone(&harness);
        handles.push(thread::spawn(move || {
            let path = make_path("hvac");
            for _ in 0..10 {
                let mut h = harness.lock().unwrap();
                let record = make_fault_record(FaultId::Numeric(0x7001), LifecycleStage::Failed);
                h.processor.process_record(&path, &record);
            }
        }));
    }

    // IVI writer thread
    {
        let harness = Arc::clone(&harness);
        handles.push(thread::spawn(move || {
            let path = make_path("ivi");
            for _ in 0..10 {
                let mut h = harness.lock().unwrap();
                let record = make_fault_record(
                    FaultId::Text(
                        common::types::to_static_short_string("ivi.display.init_timeout").unwrap(),
                    ),
                    LifecycleStage::Failed,
                );
                h.processor.process_record(&path, &record);
            }
        }));
    }

    for h in handles {
        h.join().expect("Thread should not panic");
    }

    // Verify isolation: each catalog has its own fault counts
    let h = harness.lock().unwrap();
    let hvac_faults = h.manager.get_all_faults("hvac").unwrap();
    let ivi_faults = h.manager.get_all_faults("ivi").unwrap();

    assert_eq!(hvac_faults.len(), 2, "HVAC catalog has 2 descriptors");
    assert_eq!(ivi_faults.len(), 1, "IVI catalog has 1 descriptor");

    // HVAC fault should be marked as failed
    let hvac_fault = hvac_faults.iter().find(|f| f.code == "0x7001").unwrap();
    assert_eq!(
        hvac_fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "HVAC fault should be Failed"
    );

    // IVI fault should be marked as failed
    let ivi_fault = ivi_faults
        .iter()
        .find(|f| f.code == "ivi.display.init_timeout")
        .unwrap();
    assert_eq!(
        ivi_fault.typed_status.as_ref().unwrap().test_failed,
        Some(true),
        "IVI fault should be Failed"
    );
}
