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

use crate::{FaultSinkApi, catalog::FaultCatalog, fault_manager_sink::FaultManagerSink};
use std::sync::{Arc, OnceLock, Weak};

static FAULT_SINK: OnceLock<Weak<dyn FaultSinkApi>> = OnceLock::new();
static FAULT_CATALOG: OnceLock<Weak<FaultCatalog>> = OnceLock::new();

/// FaultApi is the long-lived handle that wires a sink and logger together.
#[derive(Clone)]
pub struct FaultApi {
    _fault_sink: Arc<dyn FaultSinkApi>,
    _fault_catalog: Arc<FaultCatalog>,
}

impl FaultApi {
    pub fn new(catalog: FaultCatalog) -> FaultApi {
        // TODO: The sink should be passed as a parameter.
        let sink: Arc<dyn FaultSinkApi> = Arc::new(FaultManagerSink::new());
        let catalog = Arc::new(catalog);

        FAULT_SINK.set(Arc::downgrade(&sink)).expect("Fault Sink already initialized");
        FAULT_CATALOG.set(Arc::downgrade(&catalog)).expect("Fault catalog already initialized");
        sink.check_fault_catalog().expect("Failed to verify the catalog hash");

        FaultApi {
            _fault_sink: sink,
            _fault_catalog: catalog,
        }
    }

    pub(crate) fn get_fault_sink() -> Arc<dyn FaultSinkApi> {
        FAULT_SINK.get().and_then(|r| r.upgrade()).expect("FaultApi not initialized")
    }

    pub fn get_fault_catalog() -> Arc<FaultCatalog> {
        FAULT_CATALOG.get().and_then(|r| r.upgrade()).expect("FaultApi not initialized")
    }
}
