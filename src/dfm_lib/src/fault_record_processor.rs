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

use crate::fault_catalog_registry::FaultCatalogRegistry;
use crate::sovd_fault_storage::SovdFaultStateStorage;
use crate::sovd_fault_storage::*;
use common::fault;
use common::types::{LongString, Sha256Vec};
use log::{error, info};
use std::sync::Arc;

pub struct FaultRecordProcessor<S: SovdFaultStateStorage> {
    storage: Arc<S>,
    catalog_registry: Arc<FaultCatalogRegistry>,
}

impl<S: SovdFaultStateStorage> FaultRecordProcessor<S> {
    pub fn new(storage: Arc<S>, catalog_registry: Arc<FaultCatalogRegistry>) -> Self {
        Self { storage, catalog_registry }
    }

    pub fn process_record(&mut self, path: &LongString, record: &fault::FaultRecord) {
        let mut state = SovdFaultState::default();
        match record.lifecycle_stage {
            fault::LifecycleStage::Failed => {
                state.test_failed = true;
                state.confirmed_dtc = true;
            }
            fault::LifecycleStage::Passed => {
                state.test_failed = false;
                state.confirmed_dtc = false;
            }
            _ => todo!("Unsupported "),
        }

        // TODO: Could consume the record and avoid cloning.
        state.env_data = record.env_data.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let storage_result = self.storage.put(&path.to_string(), &record.id, state);
        info!("Fault ID {:?} stored : {:?}", record.id, storage_result);
    }

    pub fn check_hash_sum(&self, path: &LongString, hash_sum: &Sha256Vec) -> bool {
        match self.catalog_registry.get(&path.to_string()) {
            Some(catalog) => {
                let ret = catalog.config_hash() == hash_sum.to_vec();
                if !ret {
                    error!("Fault catalog hash sum error for {:?}", path.to_string());
                    error!("Expected {:?}", catalog.config_hash());
                    error!("Received {:?}", hash_sum.to_vec());
                }
                ret
            }
            None => {
                error!("Catalog hash sum entity {:?} not found ", path.to_string());
                false
            }
        }
    }
}
