// Copyright (c) 2025 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// This program and the accompanying materials are made available under the
// terms of the Apache License Version 2.0 which is available at
// <https://www.apache.org/licenses/LICENSE-2.0>
//
// SPDX-License-Identifier: Apache-2.0
//

use crate::sovd_fault_manager::SovdEnvData;
use common::{fault, types::ShortString};
use rust_kvs::prelude::*;
use serde_json::to_string;
use std::{collections::HashMap, path::Path};

// TODO: Most of the storage should be pub(crate).

#[derive(Default, Debug, PartialEq, Eq)]
pub struct SovdFaultState {
    // TODO: Just use an integer for the DTC flags.
    pub(crate) test_failed: bool,
    pub(crate) test_failed_this_operation_cycle: bool,
    pub(crate) test_failed_since_last_clear: bool,
    pub(crate) test_not_completed_this_operation_cycle: bool,
    pub(crate) test_not_completed_since_last_clear: bool,
    pub(crate) pending_dtc: bool,
    pub(crate) confirmed_dtc: bool,
    pub(crate) warning_indicator_requested: bool,
    pub(crate) env_data: SovdEnvData,
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
        map.insert("test_failed_since_last_clear".to_string(), self.test_failed_since_last_clear.to_kvs()?);
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
        map.insert("warning_indicator_requested".to_string(), self.warning_indicator_requested.to_kvs()?);
        let kvs_env_data = self
            .env_data
            .iter()
            .map(|(k, v)| (k.clone(), KvsValue::from(v.as_str())))
            .collect::<KvsMap>();
        map.insert("env_data".to_string(), kvs_env_data.to_kvs()?);
        map.to_kvs()
    }
}

impl KvsDeserialize for SovdFaultState {
    type Error = ErrorCode;

    fn from_kvs(kvs_value: &KvsValue) -> Result<Self, Self::Error> {
        if let KvsValue::Object(map) = kvs_value {
            let kvs_env_data = KvsMap::from_kvs(map.get("env_data").ok_or(ErrorCode::DeserializationFailed("".to_string()))?)?;
            let mut env_data = HashMap::new();

            for (k, v) in kvs_env_data {
                env_data.insert(k, String::from_kvs(&v)?);
            }

            Ok(SovdFaultState {
                test_failed: bool::from_kvs(map.get("test_failed").ok_or(ErrorCode::DeserializationFailed("".to_string()))?)?,
                test_failed_this_operation_cycle: bool::from_kvs(
                    map.get("test_failed_this_operation_cycle")
                        .ok_or(ErrorCode::DeserializationFailed("".to_string()))?,
                )?,
                test_failed_since_last_clear: bool::from_kvs(
                    map.get("test_failed_since_last_clear")
                        .ok_or(ErrorCode::DeserializationFailed("".to_string()))?,
                )?,
                test_not_completed_this_operation_cycle: bool::from_kvs(
                    map.get("test_not_completed_this_operation_cycle")
                        .ok_or(ErrorCode::DeserializationFailed("".to_string()))?,
                )?,
                test_not_completed_since_last_clear: bool::from_kvs(
                    map.get("test_not_completed_since_last_clear")
                        .ok_or(ErrorCode::DeserializationFailed("".to_string()))?,
                )?,
                pending_dtc: bool::from_kvs(map.get("pending_dtc").ok_or(ErrorCode::DeserializationFailed("".to_string()))?)?,
                confirmed_dtc: bool::from_kvs(map.get("confirmed_dtc").ok_or(ErrorCode::DeserializationFailed("".to_string()))?)?,
                warning_indicator_requested: bool::from_kvs(
                    map.get("warning_indicator_requested")
                        .ok_or(ErrorCode::DeserializationFailed("".to_string()))?,
                )?,
                env_data,
            })
        } else {
            Err(ErrorCode::DeserializationFailed("".to_string()))
        }
    }
}

pub trait SovdFaultStateStorage: Send + Sync {
    // TODO: Return Result instead of a bool or Option.
    fn put(&self, path: &str, fault_id: &fault::FaultId, state: SovdFaultState) -> bool;
    fn get_all(&self, path: &str) -> Option<Vec<(fault::FaultId, SovdFaultState)>>;
    fn get(&self, path: &str, fault_id: &fault::FaultId) -> Option<SovdFaultState>;
    fn delete_all(&self, path: &str) -> bool;
    fn delete(&self, path: &str, fault_id: &fault::FaultId) -> bool;
}

pub struct KvsSovdFaultStateStorage {
    kvs: Kvs, /*
              "path": {
                  "fault id": FaultStatus,
                  "fault id": FaultStatus,
                  ...
              },
              "path": {
                  ...
              },
              ...
              */
}

impl KvsSovdFaultStateStorage {
    // TODO: Return Result instead of Optiona.
    pub fn new(dir: &Path, instance: usize) -> Option<Self> {
        let builder = KvsBuilder::new(InstanceId(instance))
            .backend(Box::new(JsonBackendBuilder::new().working_dir(dir.to_path_buf()).build()))
            .kvs_load(KvsLoad::Optional);
        let kvs = builder.build().expect("Failed to build Kvs");

        Some(Self { kvs })
    }
}

impl SovdFaultStateStorage for KvsSovdFaultStateStorage {
    fn put(&self, path: &str, fault_id: &fault::FaultId, state: SovdFaultState) -> bool {
        let mut states = self.kvs.get_value_as::<KvsMap>(path).unwrap_or(HashMap::new());
        states.insert(fault_id_to_key(fault_id), state.to_kvs().expect("Failed to serialize FaultState"));
        self.kvs.set_value(path, states).expect("Failed to update FaultState");

        true
    }

    fn get_all(&self, path: &str) -> Option<Vec<(fault::FaultId, SovdFaultState)>> {
        let mut result = Vec::new();

        if let Ok(states) = self.kvs.get_value_as::<KvsMap>(path) {
            for (fault_id_key, state) in &states {
                result.push((
                    fault_id_from_key(fault_id_key),
                    SovdFaultState::from_kvs(state).expect("Failed to deserialize FaultState"),
                ));
            }

            return Some(result);
        }

        None
    }

    fn get(&self, path: &str, fault_id: &fault::FaultId) -> Option<SovdFaultState> {
        let states = self.kvs.get_value_as::<KvsMap>(path).ok()?;
        let state = states.get(&fault_id_to_key(fault_id))?;
        Some(SovdFaultState::from_kvs(state).expect("Failed to deserialize FaultState"))
    }

    fn delete_all(&self, _path: &str) -> bool {
        todo!("Unsupported")
    }

    fn delete(&self, _path: &str, _fault_id: &fault::FaultId) -> bool {
        todo!("Unsupported")
    }
}

fn fault_id_to_key(fault_id: &fault::FaultId) -> String {
    match fault_id {
        fault::FaultId::Numeric(x) => to_string(x).expect("Invalid fault ID value"),
        fault::FaultId::Text(t) => t.to_string(),
        fault::FaultId::Uuid(_) => todo!("Unsupported"),
    }
}

fn fault_id_from_key(key: &str) -> fault::FaultId {
    fault::FaultId::Text(ShortString::try_from(key).expect("Failed to convert key to fault id"))
}

#[cfg(test)]
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
        };

        let state_to_kvs = state.to_kvs().unwrap();
        let state_from_kvs = SovdFaultState::from_kvs(&state_to_kvs).unwrap();

        assert_eq!(state, state_from_kvs);
    }
}
