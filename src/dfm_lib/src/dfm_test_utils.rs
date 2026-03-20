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
//! Shared test utilities for `dfm_lib` tests.
//!
//! Provides `InMemoryStorage` (a thread-safe mock implementing
//! `SovdFaultStateStorage`) and helper functions for building registries,
//! records, and paths used across all `dfm_lib` test modules.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock, mpsc},
};

use common::{
    catalog::{FaultCatalogBuilder, FaultCatalogConfig},
    config::ResetPolicy,
    debounce::DebounceMode,
    enabling_condition::EnablingConditionNotification,
    fault,
    fault::*,
    sink_error::SinkError,
    types::*,
};

use crate::{
    fault_catalog_registry::FaultCatalogRegistry,
    operation_cycle::OperationCycleTracker,
    sovd_fault_storage::{SovdFaultState, SovdFaultStateStorage, StorageError},
    transport::DfmTransport,
};

// ============================================================================
// In-memory mock storage for testing
// ============================================================================

/// Thread-safe in-memory storage implementing `SovdFaultStateStorage`.
pub struct InMemoryStorage {
    data: Mutex<HashMap<String, HashMap<String, SovdFaultState>>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl SovdFaultStateStorage for InMemoryStorage {
    fn put(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
        state: SovdFaultState,
    ) -> Result<(), StorageError> {
        let key = fault_id_to_string(fault_id);
        let mut data = self.data.lock().unwrap();
        data.entry(path.to_string()).or_default().insert(key, state);
        Ok(())
    }

    fn get_all(&self, path: &str) -> Result<Vec<(fault::FaultId, SovdFaultState)>, StorageError> {
        let data = self.data.lock().unwrap();
        match data.get(path) {
            Some(faults) => Ok(faults
                .iter()
                .map(|(key, state)| (fault_id_from_string(key), state.clone()))
                .collect()),
            None => Ok(Vec::new()),
        }
    }

    fn get(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
    ) -> Result<Option<SovdFaultState>, StorageError> {
        let key = fault_id_to_string(fault_id);
        let data = self.data.lock().unwrap();
        Ok(data.get(path).and_then(|faults| faults.get(&key).cloned()))
    }

    fn delete_all(&self, path: &str) -> Result<(), StorageError> {
        let mut data = self.data.lock().unwrap();
        data.remove(path);
        Ok(())
    }

    fn delete(&self, path: &str, fault_id: &fault::FaultId) -> Result<(), StorageError> {
        let key = fault_id_to_string(fault_id);
        let mut data = self.data.lock().unwrap();
        if let Some(faults) = data.get_mut(path)
            && faults.remove(&key).is_some()
        {
            return Ok(());
        }
        Err(StorageError::NotFound)
    }
}

/// Encode `FaultId` to a typed string key, mirroring the real storage format.
pub fn fault_id_to_string(fault_id: &fault::FaultId) -> String {
    match fault_id {
        fault::FaultId::Numeric(x) => format!("n:{x}"),
        fault::FaultId::Text(t) => format!("t:{t}"),
        fault::FaultId::Uuid(u) => {
            use core::fmt::Write;
            let hex = u.iter().fold(String::with_capacity(32), |mut acc, b| {
                let _ = write!(acc, "{b:02x}");
                acc
            });
            format!("u:{hex}")
        }
    }
}

/// Decode a typed string key back to a `FaultId`.
fn fault_id_from_string(key: &str) -> fault::FaultId {
    if let Some(num_str) = key.strip_prefix("n:")
        && let Ok(n) = num_str.parse::<u32>()
    {
        return fault::FaultId::Numeric(n);
    }
    if let Some(hex_str) = key.strip_prefix("u:")
        && hex_str.len() == 32
    {
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            if let Ok(b) = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16) {
                *byte = b;
            }
        }
        return fault::FaultId::Uuid(bytes);
    }
    if let Some(text) = key.strip_prefix("t:") {
        return fault::FaultId::Text(ShortString::try_from(text).unwrap());
    }
    // Backward compat: untyped keys → Text
    fault::FaultId::Text(ShortString::try_from(key).unwrap())
}

// ============================================================================
// Test helpers
// ============================================================================

/// Registry with `FaultId::Text` descriptors (`SovdFaultManager` requires Text IDs).
pub fn make_text_registry() -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: "test_entity".into(),
        version: 1,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("fault_a").unwrap()),
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
                id: FaultId::Text(to_static_short_string("fault_b").unwrap()),
                name: to_static_short_string("Fault B").unwrap(),
                summary: None,
                category: FaultType::Hardware,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
        ],
    };
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

/// Registry with all three `FaultId` variants (Text, Numeric, Uuid).
pub fn make_mixed_registry() -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: "mixed_entity".into(),
        version: 1,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("fault_text").unwrap()),
                name: to_static_short_string("Text Fault").unwrap(),
                summary: Some(to_static_long_string("A text-identified fault").unwrap()),
                category: FaultType::Software,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Numeric(0x1001),
                name: to_static_short_string("Numeric Fault").unwrap(),
                summary: Some(to_static_long_string("A numeric DTC-like fault").unwrap()),
                category: FaultType::Hardware,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Uuid([
                    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
                    0x0E, 0x0F, 0x10,
                ]),
                name: to_static_short_string("UUID Fault").unwrap(),
                summary: Some(to_static_long_string("A UUID-identified fault").unwrap()),
                category: FaultType::Communication,
                severity: FaultSeverity::Fatal,
                compliance: ComplianceVec::new(),
                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
        ],
    };
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

/// Registry with `FaultId::Numeric` descriptors (for processor tests only).
pub fn make_registry() -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: "test_entity".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: FaultId::Numeric(42),
            name: to_static_short_string("Test fault").unwrap(),
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
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

pub fn make_record(fault_id: FaultId, stage: LifecycleStage) -> FaultRecord {
    FaultRecord {
        id: fault_id,
        time: IpcTimestamp::default(),
        source: common::ids::SourceId {
            entity: to_static_short_string("test").unwrap(),
            ecu: None,
            domain: None,
            sw_component: None,
            instance: None,
        },
        lifecycle_phase: LifecyclePhase::Running,
        lifecycle_stage: stage,
        env_data: MetadataVec::new(),
    }
}

pub fn make_path(path: &str) -> LongString {
    LongString::from_str_truncated(path).unwrap()
}

/// Registry with manager-side debounce configured.
pub fn make_debounce_registry(
    entity_id: &str,
    fault_id: FaultId,
    debounce: DebounceMode,
) -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: entity_id.to_string().into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id,
            name: to_static_short_string("Debounced fault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: Some(debounce),
            manager_side_reset: None,
        }],
    };
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

/// Create a shared `OperationCycleTracker` for testing.
pub fn make_cycle_tracker() -> Arc<RwLock<OperationCycleTracker>> {
    Arc::new(RwLock::new(OperationCycleTracker::new()))
}

/// Registry with manager-side reset (aging) policy configured.
pub fn make_aging_registry(
    entity_id: &str,
    fault_id: FaultId,
    reset_policy: ResetPolicy,
) -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: entity_id.to_string().into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id,
            name: to_static_short_string("Aging fault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: Some(reset_policy),
        }],
    };
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

/// Registry with compliance tags set (for `warning_indicator_requested` tests).
pub fn make_compliance_registry(
    entity_id: &str,
    fault_id: FaultId,
    compliance: ComplianceVec,
) -> Arc<FaultCatalogRegistry> {
    let config = FaultCatalogConfig {
        id: entity_id.to_string().into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id,
            name: to_static_short_string("Compliance fault").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance,
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }],
    };
    let catalog = FaultCatalogBuilder::new()
        .cfg_struct(config)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog]))
}

/// Registry with two sources sharing the same fault ID but independent debounce.
pub fn make_two_source_debounce_registry(
    fault_id: FaultId,
    debounce: DebounceMode,
) -> Arc<FaultCatalogRegistry> {
    let config1 = FaultCatalogConfig {
        id: "app1".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id.clone(),
            name: to_static_short_string("Fault from app1").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: Some(debounce),
            manager_side_reset: None,
        }],
    };
    let config2 = FaultCatalogConfig {
        id: "app2".into(),
        version: 1,
        faults: vec![FaultDescriptor {
            id: fault_id,
            name: to_static_short_string("Fault from app2").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: Some(debounce),
            manager_side_reset: None,
        }],
    };
    let catalog1 = FaultCatalogBuilder::new()
        .cfg_struct(config1)
        .unwrap()
        .build();
    let catalog2 = FaultCatalogBuilder::new()
        .cfg_struct(config2)
        .unwrap()
        .build();
    Arc::new(FaultCatalogRegistry::new(vec![catalog1, catalog2]))
}

// ============================================================================
// In-memory DFM transport for testing (no iceoryx2 dependency)
// ============================================================================

/// Channel-based [`DfmTransport`] for unit tests.
///
/// Uses `mpsc` channels for events and collects published responses
/// and notifications in thread-safe vectors. No iceoryx2 shared memory
/// is required, so tests can run under Miri and in parallel without
/// `#[serial(ipc)]`.
///
/// # Example
///
/// ```rust,ignore
/// let (transport, sender) = InMemoryTransport::new();
/// sender.send(DiagnosticEvent::Fault((path, record))).unwrap();
/// // pass transport to run_dfm_loop(...)
/// ```
#[allow(dead_code)]
pub struct InMemoryTransport {
    receiver: Mutex<mpsc::Receiver<DiagnosticEvent>>,
    hash_responses: Arc<Mutex<Vec<bool>>>,
    ec_notifications: Arc<Mutex<Vec<EnablingConditionNotification>>>,
}

#[allow(dead_code)]
impl InMemoryTransport {
    /// Create a new in-memory transport and its event sender.
    ///
    /// The returned `mpsc::Sender` is used to inject events into the DFM
    /// loop (simulating IPC messages from reporter applications).
    pub fn new() -> (Self, mpsc::Sender<DiagnosticEvent>) {
        let (tx, rx) = mpsc::channel();
        let transport = Self {
            receiver: Mutex::new(rx),
            hash_responses: Arc::new(Mutex::new(Vec::new())),
            ec_notifications: Arc::new(Mutex::new(Vec::new())),
        };
        (transport, tx)
    }

    /// Get all hash responses published so far.
    pub fn hash_responses(&self) -> Vec<bool> {
        self.hash_responses.lock().unwrap().clone()
    }

    /// Get all EC notifications published so far.
    pub fn ec_notifications(&self) -> Vec<EnablingConditionNotification> {
        self.ec_notifications.lock().unwrap().clone()
    }
}

impl DfmTransport for InMemoryTransport {
    fn receive_event(&self) -> Result<Option<DiagnosticEvent>, SinkError> {
        let rx = self.receiver.lock().unwrap();
        match rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn publish_hash_response(&self, response: bool) -> Result<(), SinkError> {
        self.hash_responses.lock().unwrap().push(response);
        Ok(())
    }

    fn publish_ec_notification(
        &self,
        notification: EnablingConditionNotification,
    ) -> Result<(), SinkError> {
        self.ec_notifications.lock().unwrap().push(notification);
        Ok(())
    }

    fn wait(&self, timeout: core::time::Duration) -> Result<bool, SinkError> {
        std::thread::sleep(timeout);
        Ok(true)
    }
}
