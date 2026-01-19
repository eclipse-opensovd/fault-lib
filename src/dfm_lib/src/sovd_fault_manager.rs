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
use crate::sovd_fault_storage::*;
use common::{fault, types::ShortString};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    BadArgument,
    Generic,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SovdFault {
    pub code: String,
    pub display_code: String,
    pub scope: String,
    pub fault_name: String,
    pub fault_translation_id: String,
    pub severity: u32,
    pub status: HashMap<String, String>,
}

impl SovdFault {
    fn new(descriptor: &fault::FaultDescriptor, state: &SovdFaultState) -> Self {
        Self {
            code: fault_id_to_code(&descriptor.id),
            display_code: fault_id_to_code(&descriptor.id),
            scope: "".into(),
            fault_name: descriptor.name.to_string(),
            fault_translation_id: "".into(),
            severity: descriptor.severity as u32,
            status: HashMap::from([
                ("testFailed".into(), (state.test_failed as u32).to_string()),
                (
                    "testFailedThisOperationCycle".into(),
                    (state.test_failed_this_operation_cycle as u32).to_string(),
                ),
                ("testFailedSinceLastClear".into(), (state.test_failed_since_last_clear as u32).to_string()),
                (
                    "testNotCompletedThisOperationCycle".into(),
                    (state.test_not_completed_this_operation_cycle as u32).to_string(),
                ),
                (
                    "testNotCompletedSinceLastClear".into(),
                    (state.test_not_completed_since_last_clear as u32).to_string(),
                ),
                ("pendingDTC".into(), (state.pending_dtc as u32).to_string()),
                ("confirmedDTC".into(), (state.confirmed_dtc as u32).to_string()),
                ("warningIndicatorRequested".into(), (state.warning_indicator_requested as u32).to_string()),
            ]),
        }
    }
}

pub type SovdEnvData = HashMap<String, String>;

pub struct SovdFaultManager<S: SovdFaultStateStorage> {
    storage: Arc<S>,
    registry: Arc<FaultCatalogRegistry>,
}

impl<S: SovdFaultStateStorage> SovdFaultManager<S> {
    pub fn new(storage: Arc<S>, registry: Arc<FaultCatalogRegistry>) -> Self {
        Self { storage, registry }
    }

    pub fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        let Some(catalog) = self.registry.catalogs.get(path) else {
            return Err(Error::BadArgument);
        };
        let descriptors = catalog.descriptors();
        let mut faults = Vec::new();

        for descriptor in descriptors {
            // Right now all faults are always returned. Faults for which a record wasn't received are returned with a clear status.
            // TODO: Need to decide if this is the correct behavior. It could be that a fault shouldn't be reported once Passed.
            let state = self.storage.get(path, &descriptor.id).unwrap_or(SovdFaultState::default());

            faults.push(SovdFault::new(descriptor, &state));
        }

        Ok(faults)
    }

    pub fn get_fault(&self, path: &str, fault_code: &str) -> Result<(SovdFault, SovdEnvData), Error> {
        let Some(catalog) = self.registry.catalogs.get(path) else {
            return Err(Error::BadArgument);
        };
        let fault_id = fault_id_from_code(fault_code);
        let Some(descriptor) = catalog.descriptor(&fault_id) else {
            return Err(Error::Generic);
        };
        // Faults for which a record wasn't received are returned with a clear status.
        // TODO: Need to decide if this is the correct behavior. It could be that a fault shouldn't be reported once Passed.
        let state = self.storage.get(path, &fault_id).unwrap_or_default();

        Ok((SovdFault::new(descriptor, &state), state.env_data))
    }

    pub fn delete_all_faults(&mut self, _path: &str) -> Result<(), Error> {
        todo!("Not implemented");
    }

    pub fn delete_fault(&mut self, _path: &str, _fault_code: &str) -> Result<(), Error> {
        todo!("Not implemented");
    }
}

fn fault_id_to_code(fault_id: &fault::FaultId) -> String {
    match fault_id {
        fault::FaultId::Numeric(_) => todo!("Unsupported"),
        fault::FaultId::Text(t) => t.to_string(),
        fault::FaultId::Uuid(_) => todo!("Unsupported"),
    }
}

fn fault_id_from_code(fault_code: &str) -> fault::FaultId {
    fault::FaultId::Text(ShortString::try_from(fault_code).expect("Failed to convert fault code to fault id"))
}
