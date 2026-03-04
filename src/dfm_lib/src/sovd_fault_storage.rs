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

//! Persistent storage layer for SOVD fault state.
//!
//! Defines the [`SovdFaultStateStorage`] trait and a `rust_kvs`-backed
//! implementation for durable fault-state persistence across DFM restarts.
//! Fault states, environment snapshots, and DTC status bits are stored
//! and retrieved through this abstraction.

use crate::sovd_fault_manager::SovdEnvData;
use common::{fault, types::ShortString};
use rust_kvs::prelude::*;
use std::{collections::HashMap, path::Path, sync::Mutex};

/// Errors that can occur during fault state storage operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Serialization of fault state data failed.
    #[error("serialization failed: {0}")]
    Serialization(String),

    /// Deserialization of fault state data failed.
    #[error("deserialization failed: {0}")]
    Deserialization(String),

    /// Backend storage operation failed.
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Initialization of storage backend failed.
    #[error("storage initialization failed: {0}")]
    Init(String),

    /// Requested entry was not found.
    #[error("entry not found")]
    NotFound,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct SovdFaultState {
    // Individual boolean fields are used instead of a packed integer for type
    // safety and readability. The integer representation is available via
    // `SovdFaultStatus::compute_mask()` when needed (e.g. for UDS/ISO 14229).
    pub(crate) test_failed: bool,
    pub(crate) test_failed_this_operation_cycle: bool,
    pub(crate) test_failed_since_last_clear: bool,
    pub(crate) test_not_completed_this_operation_cycle: bool,
    pub(crate) test_not_completed_since_last_clear: bool,
    pub(crate) pending_dtc: bool,
    pub(crate) confirmed_dtc: bool,
    pub(crate) warning_indicator_requested: bool,
    pub(crate) env_data: SovdEnvData,

    // Aging counters (for reset/healing tracking)
    /// Number of times this fault has occurred.
    pub(crate) occurrence_counter: u32,
    /// Number of aging cycles passed (used for aging logic).
    pub(crate) aging_counter: u32,
    /// Number of times this fault was healed/reset.
    pub(crate) healing_counter: u32,
    /// Unix timestamp (secs) of first occurrence.
    pub(crate) first_occurrence_secs: u64,
    /// Unix timestamp (secs) of most recent occurrence.
    pub(crate) last_occurrence_secs: u64,
}

impl SovdFaultState {
    /// Record a new fault occurrence: increment counter and update timestamps.
    pub(crate) fn record_occurrence(&mut self, now_secs: u64) {
        self.occurrence_counter = self.occurrence_counter.saturating_add(1);
        if self.first_occurrence_secs == 0 {
            self.first_occurrence_secs = now_secs;
        }
        self.last_occurrence_secs = now_secs;
    }
}

impl KvsSerialize for SovdFaultState {
    type Error = ErrorCode;

    fn to_kvs(&self) -> Result<KvsValue, Self::Error> {
        let mut map = KvsMap::new();
        map.insert("test_failed".to_string(), self.test_failed.to_kvs()?);
        map.insert(
            "test_failed_this_operation_cycle".to_string(),
            self.test_failed_this_operation_cycle.to_kvs()?,
        );
        map.insert(
            "test_failed_since_last_clear".to_string(),
            self.test_failed_since_last_clear.to_kvs()?,
        );
        map.insert(
            "test_not_completed_this_operation_cycle".to_string(),
            self.test_not_completed_this_operation_cycle.to_kvs()?,
        );
        map.insert(
            "test_not_completed_since_last_clear".to_string(),
            self.test_not_completed_since_last_clear.to_kvs()?,
        );
        map.insert("pending_dtc".to_string(), self.pending_dtc.to_kvs()?);
        map.insert("confirmed_dtc".to_string(), self.confirmed_dtc.to_kvs()?);
        map.insert(
            "warning_indicator_requested".to_string(),
            self.warning_indicator_requested.to_kvs()?,
        );
        let kvs_env_data = self
            .env_data
            .iter()
            .map(|(k, v)| (k.clone(), KvsValue::from(v.as_str())))
            .collect::<KvsMap>();
        map.insert("env_data".to_string(), kvs_env_data.to_kvs()?);

        // Aging counters — propagate error on out-of-range values rather
        // than silently clamping. Silent fallback could mask data corruption
        // in long-running automotive systems.
        map.insert(
            "occurrence_counter".to_string(),
            KvsValue::from(i64::from(self.occurrence_counter)),
        );
        map.insert(
            "aging_counter".to_string(),
            KvsValue::from(i64::from(self.aging_counter)),
        );
        map.insert(
            "healing_counter".to_string(),
            KvsValue::from(i64::from(self.healing_counter)),
        );
        map.insert(
            "first_occurrence_secs".to_string(),
            KvsValue::from(i64::try_from(self.first_occurrence_secs).map_err(|_| {
                ErrorCode::SerializationFailed("first_occurrence_secs out of i64 range".to_string())
            })?),
        );
        map.insert(
            "last_occurrence_secs".to_string(),
            KvsValue::from(i64::try_from(self.last_occurrence_secs).map_err(|_| {
                ErrorCode::SerializationFailed("last_occurrence_secs out of i64 range".to_string())
            })?),
        );

        map.to_kvs()
    }
}

impl KvsDeserialize for SovdFaultState {
    type Error = ErrorCode;

    #[allow(clippy::items_after_statements)]
    fn from_kvs(kvs_value: &KvsValue) -> Result<Self, Self::Error> {
        if let KvsValue::Object(map) = kvs_value {
            let kvs_env_data = KvsMap::from_kvs(
                map.get("env_data")
                    .ok_or(ErrorCode::DeserializationFailed("env_data".to_string()))?,
            )?;
            let mut env_data = HashMap::new();

            for (k, v) in kvs_env_data {
                env_data.insert(k, String::from_kvs(&v)?);
            }

            // Helper to extract u32/u64 with TryFrom — propagates error
            // on negative or overflowing values to surface data corruption.
            // Missing keys default to 0 for backward compatibility with
            // storage created before these fields existed.
            fn get_u32(map: &KvsMap, key: &str) -> Result<u32, ErrorCode> {
                match map.get(key) {
                    Some(v) => {
                        let i = i64::from_kvs(v).map_err(|_| {
                            ErrorCode::DeserializationFailed(format!("{key}: not an i64"))
                        })?;
                        u32::try_from(i).map_err(|_| {
                            ErrorCode::DeserializationFailed(format!(
                                "{key}: value {i} out of u32 range"
                            ))
                        })
                    }
                    None => Ok(0),
                }
            }
            fn get_u64(map: &KvsMap, key: &str) -> Result<u64, ErrorCode> {
                match map.get(key) {
                    Some(v) => {
                        let i = i64::from_kvs(v).map_err(|_| {
                            ErrorCode::DeserializationFailed(format!("{key}: not an i64"))
                        })?;
                        u64::try_from(i).map_err(|_| {
                            ErrorCode::DeserializationFailed(format!(
                                "{key}: value {i} out of u64 range"
                            ))
                        })
                    }
                    None => Ok(0),
                }
            }

            Ok(SovdFaultState {
                test_failed: bool::from_kvs(
                    map.get("test_failed")
                        .ok_or(ErrorCode::DeserializationFailed("test_failed".to_string()))?,
                )?,
                test_failed_this_operation_cycle: bool::from_kvs(
                    map.get("test_failed_this_operation_cycle").ok_or(
                        ErrorCode::DeserializationFailed(
                            "test_failed_this_operation_cycle".to_string(),
                        ),
                    )?,
                )?,
                test_failed_since_last_clear: bool::from_kvs(
                    map.get("test_failed_since_last_clear").ok_or(
                        ErrorCode::DeserializationFailed(
                            "test_failed_since_last_clear".to_string(),
                        ),
                    )?,
                )?,
                test_not_completed_this_operation_cycle: bool::from_kvs(
                    map.get("test_not_completed_this_operation_cycle").ok_or(
                        ErrorCode::DeserializationFailed(
                            "test_not_completed_this_operation_cycle".to_string(),
                        ),
                    )?,
                )?,
                test_not_completed_since_last_clear: bool::from_kvs(
                    map.get("test_not_completed_since_last_clear").ok_or(
                        ErrorCode::DeserializationFailed(
                            "test_not_completed_since_last_clear".to_string(),
                        ),
                    )?,
                )?,
                pending_dtc: bool::from_kvs(
                    map.get("pending_dtc")
                        .ok_or(ErrorCode::DeserializationFailed("pending_dtc".to_string()))?,
                )?,
                confirmed_dtc: bool::from_kvs(map.get("confirmed_dtc").ok_or(
                    ErrorCode::DeserializationFailed("confirmed_dtc".to_string()),
                )?)?,
                warning_indicator_requested: bool::from_kvs(
                    map.get("warning_indicator_requested").ok_or(
                        ErrorCode::DeserializationFailed("warning_indicator_requested".to_string()),
                    )?,
                )?,
                env_data,
                // Aging counters — propagate deserialization errors
                occurrence_counter: get_u32(map, "occurrence_counter")?,
                aging_counter: get_u32(map, "aging_counter")?,
                healing_counter: get_u32(map, "healing_counter")?,
                first_occurrence_secs: get_u64(map, "first_occurrence_secs")?,
                last_occurrence_secs: get_u64(map, "last_occurrence_secs")?,
            })
        } else {
            Err(ErrorCode::DeserializationFailed(
                "expected KvsValue::Object for SovdFaultState".to_string(),
            ))
        }
    }
}

/// Trait abstracting persistent storage of SOVD fault state.
///
/// # Errors
///
/// All methods return [`StorageError`] on serialization, deserialization,
/// backend, or not-found failures.
pub trait SovdFaultStateStorage: Send + Sync {
    /// Store (upsert) fault state for the given path and fault ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the write fails.
    fn put(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
        state: SovdFaultState,
    ) -> Result<(), StorageError>;
    /// Retrieve all fault states for a given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the read fails.
    fn get_all(&self, path: &str) -> Result<Vec<(fault::FaultId, SovdFaultState)>, StorageError>;
    /// Retrieve a single fault state by path and fault ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the read fails.
    fn get(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
    ) -> Result<Option<SovdFaultState>, StorageError>;
    /// Delete all fault states for a given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the delete fails.
    fn delete_all(&self, path: &str) -> Result<(), StorageError>;
    /// Delete a single fault state by path and fault ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the delete fails.
    fn delete(&self, path: &str, fault_id: &fault::FaultId) -> Result<(), StorageError>;
}

pub struct KvsSovdFaultStateStorage {
    kvs: Mutex<Kvs>,
}

impl KvsSovdFaultStateStorage {
    /// Create a new KVS-backed storage at the given path.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Init`] if the KVS backend cannot be created.
    ///
    /// # Instance pool
    ///
    /// KVS uses a process-global instance pool with a hard limit of
    /// `KVS_MAX_INSTANCES` (currently 10). Each `instance` ID must be
    /// unique within the process. Exceeding the limit or reusing an
    /// instance ID with a different backend path causes
    /// `InstanceParametersMismatch` errors.
    pub fn new(dir: &Path, instance: usize) -> Result<Self, StorageError> {
        let builder = KvsBuilder::new(InstanceId(instance))
            .backend(Box::new(
                JsonBackendBuilder::new()
                    .working_dir(dir.to_path_buf())
                    .build(),
            ))
            .kvs_load(KvsLoad::Optional);
        let kvs = builder
            .build()
            .map_err(|e| StorageError::Init(format!("{e:?}")))?;

        Ok(Self {
            kvs: Mutex::new(kvs),
        })
    }
}

impl SovdFaultStateStorage for KvsSovdFaultStateStorage {
    fn put(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
        state: SovdFaultState,
    ) -> Result<(), StorageError> {
        let kvs = self
            .kvs
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock poisoned: {e}")))?;
        let mut states = kvs.get_value_as::<KvsMap>(path).unwrap_or_default();
        states.insert(
            fault_id_to_key(fault_id),
            state
                .to_kvs()
                .map_err(|e| StorageError::Serialization(format!("{e:?}")))?,
        );
        kvs.set_value(path, states)
            .map_err(|e| StorageError::Backend(format!("{e:?}")))?;
        Ok(())
    }

    fn get_all(&self, path: &str) -> Result<Vec<(fault::FaultId, SovdFaultState)>, StorageError> {
        let kvs = self
            .kvs
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock poisoned: {e}")))?;
        let Ok(states) = kvs.get_value_as::<KvsMap>(path) else {
            return Ok(Vec::new());
        };
        let mut result = Vec::new();
        for (fault_id_key, state) in &states {
            let fault_state = SovdFaultState::from_kvs(state)
                .map_err(|e| StorageError::Deserialization(format!("{e:?}")))?;
            result.push((fault_id_from_key(fault_id_key)?, fault_state));
        }
        Ok(result)
    }

    fn get(
        &self,
        path: &str,
        fault_id: &fault::FaultId,
    ) -> Result<Option<SovdFaultState>, StorageError> {
        let kvs = self
            .kvs
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock poisoned: {e}")))?;
        let Ok(states) = kvs.get_value_as::<KvsMap>(path) else {
            return Ok(None);
        };
        match states.get(&fault_id_to_key(fault_id)) {
            Some(state) => {
                let fault_state = SovdFaultState::from_kvs(state)
                    .map_err(|e| StorageError::Deserialization(format!("{e:?}")))?;
                Ok(Some(fault_state))
            }
            None => Ok(None),
        }
    }

    fn delete_all(&self, path: &str) -> Result<(), StorageError> {
        let kvs = self
            .kvs
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock poisoned: {e}")))?;
        kvs.remove_key(path)
            .map_err(|e| StorageError::Backend(format!("{e:?}")))?;
        Ok(())
    }

    fn delete(&self, path: &str, fault_id: &fault::FaultId) -> Result<(), StorageError> {
        let kvs = self
            .kvs
            .lock()
            .map_err(|e| StorageError::Backend(format!("lock poisoned: {e}")))?;
        let mut states = kvs
            .get_value_as::<KvsMap>(path)
            .map_err(|e| StorageError::Backend(format!("{e:?}")))?;
        let key = fault_id_to_key(fault_id);
        if states.remove(&key).is_some() {
            kvs.set_value(path, states)
                .map_err(|e| StorageError::Backend(format!("{e:?}")))?;
            Ok(())
        } else {
            Err(StorageError::NotFound)
        }
    }
}

/// Encode a `FaultId` as a typed storage key.
///
/// Format: `n:<decimal>` for Numeric, `t:<text>` for Text, `u:<hex32>` for Uuid.
/// This preserves variant type information for lossless roundtrips.
fn fault_id_to_key(fault_id: &fault::FaultId) -> String {
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

/// Decode a storage key back to a `FaultId`.
///
/// Supports typed prefix format (`n:`, `t:`, `u:`) for new entries.
/// Unrecognized keys (from older storage) are treated as `FaultId::Text`
/// for backward compatibility.
#[allow(clippy::arithmetic_side_effects)]
fn fault_id_from_key(key: &str) -> Result<fault::FaultId, StorageError> {
    if let Some(num_str) = key.strip_prefix("n:") {
        let n = num_str
            .parse::<u32>()
            .map_err(|_| StorageError::Deserialization(format!("invalid numeric key: {key}")))?;
        return Ok(fault::FaultId::Numeric(n));
    }
    if let Some(hex_str) = key.strip_prefix("u:") {
        if hex_str.len() != 32 {
            return Err(StorageError::Deserialization(format!(
                "invalid uuid key length (expected 32 hex chars): {key}"
            )));
        }
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex_str[i * 2..i * 2 + 2], 16)
                .map_err(|_| StorageError::Deserialization(format!("invalid uuid hex: {key}")))?;
        }
        return Ok(fault::FaultId::Uuid(bytes));
    }
    if let Some(text) = key.strip_prefix("t:") {
        let short = ShortString::try_from(text)
            .map_err(|_| StorageError::Deserialization(format!("fault id key too long: {key}")))?;
        return Ok(fault::FaultId::Text(short));
    }
    // Backward compatibility: untyped keys from older storage are treated as Text.
    let short = ShortString::try_from(key)
        .map_err(|_| StorageError::Deserialization(format!("fault id key too long: {key}")))?;
    Ok(fault::FaultId::Text(short))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::unreadable_literal)]
mod tests {
    use super::*;

    #[test]
    fn to_and_from_kvs() {
        let state = SovdFaultState {
            test_failed: true,
            test_failed_this_operation_cycle: false,
            test_failed_since_last_clear: true,
            test_not_completed_this_operation_cycle: true,
            test_not_completed_since_last_clear: false,
            pending_dtc: false,
            confirmed_dtc: false,
            warning_indicator_requested: true,
            env_data: SovdEnvData::from([
                ("key1".into(), "val1".into()),
                ("key2".into(), "val2".into()),
                ("key3".into(), "val3".into()),
            ]),
            occurrence_counter: 5,
            aging_counter: 2,
            healing_counter: 1,
            first_occurrence_secs: 1700000000,
            last_occurrence_secs: 1700001000,
        };

        let state_to_kvs = state.to_kvs().unwrap();
        let state_from_kvs = SovdFaultState::from_kvs(&state_to_kvs).unwrap();

        assert_eq!(state, state_from_kvs);
    }

    // ==================== Typed key roundtrip tests ====================

    #[test]
    fn fault_id_key_roundtrip_numeric() {
        let id = fault::FaultId::Numeric(42);
        let key = fault_id_to_key(&id);
        assert_eq!(key, "n:42");
        let parsed = fault_id_from_key(&key).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn fault_id_key_roundtrip_text() {
        let id = fault::FaultId::Text(ShortString::try_from("test_fault").unwrap());
        let key = fault_id_to_key(&id);
        assert_eq!(key, "t:test_fault");
        let parsed = fault_id_from_key(&key).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn fault_id_key_roundtrip_uuid() {
        let id = fault::FaultId::Uuid([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        let key = fault_id_to_key(&id);
        assert_eq!(key, "u:0102030405060708090a0b0c0d0e0f10");
        let parsed = fault_id_from_key(&key).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn fault_id_key_backward_compat_untyped() {
        // Old-format keys without a type prefix are treated as Text.
        let parsed = fault_id_from_key("old_fault_key").unwrap();
        assert_eq!(
            parsed,
            fault::FaultId::Text(ShortString::try_from("old_fault_key").unwrap())
        );
    }

    #[test]
    fn fault_id_key_invalid_uuid_length() {
        let result = fault_id_from_key("u:0102");
        assert!(result.is_err());
    }

    #[test]
    fn fault_id_key_invalid_numeric() {
        let result = fault_id_from_key("n:not_a_number");
        assert!(result.is_err());
    }

    #[test]
    fn backward_compat_missing_aging_fields() {
        // Simulate old KVS data without aging counters
        let mut map = KvsMap::new();
        map.insert("test_failed".to_string(), true.to_kvs().unwrap());
        map.insert(
            "test_failed_this_operation_cycle".to_string(),
            false.to_kvs().unwrap(),
        );
        map.insert(
            "test_failed_since_last_clear".to_string(),
            false.to_kvs().unwrap(),
        );
        map.insert(
            "test_not_completed_this_operation_cycle".to_string(),
            false.to_kvs().unwrap(),
        );
        map.insert(
            "test_not_completed_since_last_clear".to_string(),
            false.to_kvs().unwrap(),
        );
        map.insert("pending_dtc".to_string(), false.to_kvs().unwrap());
        map.insert("confirmed_dtc".to_string(), true.to_kvs().unwrap());
        map.insert(
            "warning_indicator_requested".to_string(),
            false.to_kvs().unwrap(),
        );
        map.insert("env_data".to_string(), KvsMap::new().to_kvs().unwrap());

        let kvs_value = map.to_kvs().unwrap();
        let state = SovdFaultState::from_kvs(&kvs_value).unwrap();

        // New aging fields should default to 0
        assert_eq!(state.occurrence_counter, 0);
        assert_eq!(state.aging_counter, 0);
        assert_eq!(state.healing_counter, 0);
        assert_eq!(state.first_occurrence_secs, 0);
        assert_eq!(state.last_occurrence_secs, 0);
        // Old fields should still work
        assert!(state.test_failed);
        assert!(state.confirmed_dtc);
    }
}

#[cfg(test)]
mod storage_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

    use crate::dfm_test_utils::InMemoryStorage;
    use crate::sovd_fault_storage::{SovdFaultState, SovdFaultStateStorage};
    use common::fault::*;

    /// Storage stores and retrieves fault state correctly.
    #[test]
    fn storage_put_and_get() {
        let storage = InMemoryStorage::new();

        let state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            ..Default::default()
        };

        storage
            .put("entity/1", &FaultId::Numeric(42), state)
            .unwrap();

        let retrieved = storage.get("entity/1", &FaultId::Numeric(42)).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert!(retrieved.test_failed);
        assert!(retrieved.confirmed_dtc);
    }

    /// Storage returns None for non-existent entries.
    #[test]
    fn storage_get_nonexistent() {
        let storage = InMemoryStorage::new();
        assert!(
            storage
                .get("entity/1", &FaultId::Numeric(42))
                .unwrap()
                .is_none()
        );
    }

    /// Storage `get_all` returns all faults for a path.
    #[test]
    fn storage_get_all() {
        let storage = InMemoryStorage::new();

        storage
            .put(
                "entity/1",
                &FaultId::Numeric(1),
                SovdFaultState {
                    test_failed: true,
                    ..Default::default()
                },
            )
            .unwrap();
        storage
            .put(
                "entity/1",
                &FaultId::Numeric(2),
                SovdFaultState {
                    test_failed: false,
                    ..Default::default()
                },
            )
            .unwrap();

        let all = storage.get_all("entity/1").unwrap();
        assert_eq!(all.len(), 2);
    }

    /// Storage isolates different paths.
    #[test]
    fn storage_path_isolation() {
        let storage = InMemoryStorage::new();

        storage
            .put("entity/1", &FaultId::Numeric(1), SovdFaultState::default())
            .unwrap();
        storage
            .put("entity/2", &FaultId::Numeric(2), SovdFaultState::default())
            .unwrap();

        let e1 = storage.get_all("entity/1").unwrap();
        let e2 = storage.get_all("entity/2").unwrap();
        assert_eq!(e1.len(), 1);
        assert_eq!(e2.len(), 1);

        assert!(storage.get_all("entity/3").unwrap().is_empty());
    }

    /// Storage `delete_all` removes all faults for a path.
    #[test]
    fn storage_delete_all() {
        let storage = InMemoryStorage::new();

        storage
            .put("entity/1", &FaultId::Numeric(1), SovdFaultState::default())
            .unwrap();
        storage
            .put("entity/1", &FaultId::Numeric(2), SovdFaultState::default())
            .unwrap();

        storage.delete_all("entity/1").unwrap();
        assert!(storage.get_all("entity/1").unwrap().is_empty());
    }

    /// Storage delete removes a single fault.
    #[test]
    fn storage_delete_single() {
        let storage = InMemoryStorage::new();

        storage
            .put("entity/1", &FaultId::Numeric(1), SovdFaultState::default())
            .unwrap();
        storage
            .put("entity/1", &FaultId::Numeric(2), SovdFaultState::default())
            .unwrap();

        storage.delete("entity/1", &FaultId::Numeric(1)).unwrap();
        assert!(
            storage
                .get("entity/1", &FaultId::Numeric(1))
                .unwrap()
                .is_none()
        );
        assert!(
            storage
                .get("entity/1", &FaultId::Numeric(2))
                .unwrap()
                .is_some()
        );
    }
}
