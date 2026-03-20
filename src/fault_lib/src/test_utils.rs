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
// Test utilities are exclusively used by tests — unwrap/expect is acceptable.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use alloc::string::String;
use core::{
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
    time::Duration,
};
use std::sync::Mutex;

use common::{
    config::ResetPolicy, debounce::DebounceMode, fault::*, ids::*, sink_error::SinkError, types::*,
};

use crate::{reporter::ReporterConfig, sink::FaultSinkApi};

// ============================================================================
// IPC Test Isolation Helpers
// ============================================================================

/// Monotonic counter ensuring unique service names across all tests in a process.
static IPC_TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Generate a unique iceoryx2 service name for test isolation.
///
/// Each call returns a distinct name incorporating the process ID and an
/// atomic counter, preventing shared-memory conflicts when tests run in
/// parallel or when stale resources linger from a previous run.
///
/// # Example
///
/// ```ignore
/// let svc = unique_ipc_service_name("worker_start");
/// // → "test/worker_start/12345/0"
/// ```
pub fn unique_ipc_service_name(prefix: &str) -> String {
    let id = IPC_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    format!("test/{prefix}/{pid}/{id}")
}

#[allow(dead_code)]
/// # Panics
///
/// Panics if the static string conversion fails (should never happen
/// with hard-coded test values).
#[must_use]
pub fn stub_source() -> SourceId {
    SourceId {
        entity: to_static_short_string("source").unwrap(),
        ecu: Some(ShortString::from_bytes("ECU-A".as_bytes()).unwrap()),
        domain: Some(to_static_short_string("ADAS").unwrap()),
        sw_component: Some(to_static_short_string("Perception").unwrap()),
        instance: Some(to_static_short_string("0").unwrap()),
    }
}

#[allow(dead_code)]
#[must_use]
pub fn stub_config() -> ReporterConfig {
    ReporterConfig {
        source: stub_source(),
        lifecycle_phase: LifecyclePhase::Running,
        default_env_data: MetadataVec::new(),
    }
}

#[allow(dead_code)]
#[must_use]
pub fn stub_descriptor(
    id: FaultId,
    name: ShortString,
    debounce: Option<DebounceMode>,
    reset: Option<ResetPolicy>,
) -> FaultDescriptor {
    FaultDescriptor {
        id,
        name,
        summary: None,
        category: FaultType::Software,
        severity: FaultSeverity::Warn,
        compliance: ComplianceVec::new(),
        reporter_side_debounce: debounce,
        reporter_side_reset: reset,
        manager_side_debounce: None,
        manager_side_reset: None,
    }
}

#[allow(dead_code)]
#[must_use]
pub fn stub_record(desc: FaultDescriptor) -> FaultRecord {
    FaultRecord {
        id: desc.id,
        time: IpcTimestamp::default(),
        source: stub_source(),
        lifecycle_phase: LifecyclePhase::Running,
        lifecycle_stage: LifecycleStage::NotTested,
        env_data: MetadataVec::new(),
    }
}

/// # Panics
///
/// Panics if the test descriptors cannot be constructed (should never
/// happen with hard-coded values).
#[must_use]
pub fn create_dummy_descriptors() -> Vec<FaultDescriptor> {
    let d1 = FaultDescriptor {
        id: FaultId::Text(to_static_short_string("d1").unwrap()),

        name: to_static_short_string("Descriptor 1").unwrap(),
        summary: None,

        category: FaultType::Software,
        severity: FaultSeverity::Debug,
        compliance: ComplianceVec::try_from(
            &[
                ComplianceTag::EmissionRelevant,
                ComplianceTag::SafetyCritical,
            ][..],
        )
        .unwrap(),

        reporter_side_debounce: Some(DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100u64).into(),
        }),
        reporter_side_reset: None,
        manager_side_debounce: None,
        manager_side_reset: None,
    };

    let d2 = FaultDescriptor {
        id: FaultId::Text(to_static_short_string("d2").unwrap()),

        name: to_static_short_string("Descriptor 2").unwrap(),
        summary: Some(to_static_long_string("Human-readable summary").unwrap()),

        category: FaultType::Configuration,
        severity: FaultSeverity::Warn,
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
            cooldown: Duration::from_millis(100u64).into(),
        }),
        manager_side_reset: None,
    };
    vec![d1, d2]
}

/// # Panics
///
/// Panics if serialization of the dummy descriptors fails.
#[must_use]
pub fn load_dummy_config_file() -> String {
    serde_json::to_string(&create_dummy_descriptors()).expect("serde_json::to_string failed")
}

// ============================================================================
// Mock Sink Implementations (implement FaultSinkApi trait)
// ============================================================================

/// Sink that records all published `FaultRecords` for test assertions.
#[allow(dead_code)]
pub struct RecordingSink {
    records: Mutex<Vec<FaultRecord>>,
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl RecordingSink {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Mutex::new(vec![]),
        }
    }

    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn received_records(&self) -> Vec<FaultRecord> {
        self.records.lock().expect("RecordingSink poisoned").clone()
    }

    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn count(&self) -> usize {
        self.records.lock().expect("RecordingSink poisoned").len()
    }
}

impl FaultSinkApi for RecordingSink {
    fn publish(&self, _path: &str, record: FaultRecord) -> Result<(), SinkError> {
        self.records
            .lock()
            .expect("RecordingSink poisoned")
            .push(record);
        Ok(())
    }
    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        Ok(true)
    }
}

/// Sink that simulates slow transport for non-blocking tests (REQ-6).
#[allow(dead_code)]
pub struct SlowSink {
    delay: Duration,
}

#[allow(dead_code)]
impl SlowSink {
    #[must_use]
    pub fn new(delay: Duration) -> Self {
        Self { delay }
    }
}

impl FaultSinkApi for SlowSink {
    fn publish(&self, _path: &str, _record: FaultRecord) -> Result<(), SinkError> {
        std::thread::sleep(self.delay);
        Ok(())
    }
    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        Ok(true)
    }
}

/// Thread-safe atomic counter sink for concurrent access tests.
#[allow(dead_code)]
pub struct AtomicCountingSink {
    count: AtomicUsize,
}

impl Default for AtomicCountingSink {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl AtomicCountingSink {
    #[must_use]
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }
    pub fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}

impl FaultSinkApi for AtomicCountingSink {
    fn publish(&self, _path: &str, _record: FaultRecord) -> Result<(), SinkError> {
        self.count.fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        Ok(true)
    }
}

/// Sink that always returns a specified error (for error path tests).
#[allow(dead_code)]
pub struct FailingSink {
    error: SinkError,
}

#[allow(dead_code)]
impl FailingSink {
    #[must_use]
    pub fn new(error: SinkError) -> Self {
        Self { error }
    }

    #[must_use]
    pub fn transport_down() -> Self {
        Self::new(SinkError::TransportDown)
    }

    #[must_use]
    pub fn timeout() -> Self {
        Self::new(SinkError::Timeout)
    }
}

impl FaultSinkApi for FailingSink {
    fn publish(&self, _path: &str, _record: FaultRecord) -> Result<(), SinkError> {
        Err(self.error.clone())
    }
    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        Err(self.error.clone())
    }
}
