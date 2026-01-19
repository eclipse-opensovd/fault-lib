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

use crate::{
    fault_catalog_registry::FaultCatalogRegistry, fault_lib_communicator::FaultLibCommunicator, fault_record_processor::FaultRecordProcessor,
    sovd_fault_manager::SovdFaultManager, sovd_fault_storage::SovdFaultStateStorage,
};
use log::error;
use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

pub struct DiagnosticFaultManager<S: SovdFaultStateStorage> {
    fault_lib_receiver_thread: Option<JoinHandle<()>>,
    // TODO: we will also need the SOVD server connection to be added here and in the builder
    storage: Arc<S>,
    registry: Arc<FaultCatalogRegistry>,
}

impl<S: SovdFaultStateStorage + 'static> DiagnosticFaultManager<S> {
    pub fn new(storage: S, registry: FaultCatalogRegistry) -> Self {
        let storage = Arc::new(storage);
        let registry = Arc::new(registry);
        let processor = FaultRecordProcessor::new(Arc::clone(&storage), Arc::clone(&registry));

        // TODO: Add a SOVD server connection builder and pass the result as parameter
        let handle: JoinHandle<()> = thread::Builder::new()
            .name("fault_lib_receiver_thread".into())
            .spawn(move || {
                FaultLibCommunicator::new().run(processor);
            })
            .expect("Failed to spawn the fault_lib_receiver_thread");

        Self {
            fault_lib_receiver_thread: Some(handle),
            storage,
            registry,
        }
    }

    pub fn get_sovd_fault_manager(&self) -> SovdFaultManager<S> {
        SovdFaultManager::new(Arc::clone(&self.storage), Arc::clone(&self.registry))
    }
}

impl<S: SovdFaultStateStorage> Drop for DiagnosticFaultManager<S> {
    fn drop(&mut self) {
        println!("Joining fault_lib_receiver_thread");
        let handle = self.fault_lib_receiver_thread.take().unwrap();
        if let Err(err) = handle.join() {
            error!("fault_lib_receiver_thread panicked: {:?}", err);
        }
        println!("fault_lib_receiver_thread done!");
    }
}
