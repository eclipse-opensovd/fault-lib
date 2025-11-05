/*
* Copyright (c) 2025 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
*
* See the NOTICE file(s) distributed with this work for additional
* information regarding copyright ownership.
*
* This program and the accompanying materials are made available under the
* terms of the Apache License Version 2.0 which is available at
* https://www.apache.org/licenses/LICENSE-2.0
*
* SPDX-License-Identifier: Apache-2.0
*/

use crate::{
    catalog::FaultCatalog,
    config::ReporterConfig,
    model::FaultRecord,
    sink::{FaultSink, LogHook},
};
use std::sync::Arc;

#[derive(Clone)]
// FaultApi is the long-lived handle that wires a sink and logger together.
pub struct FaultApi {
    sink: Arc<dyn FaultSink>,
    logger: Arc<dyn LogHook>,
}

impl FaultApi {
    // Callers construct this once at bootstrap and share it across tasks.
    pub fn new(sink: Arc<dyn FaultSink>, logger: Arc<dyn LogHook>) -> Self {
        Self { sink, logger }
    }

    /// Report an occurrence of a fault. Always logs via LogHook, then publishes via sink.
    /// or in other words: enqueue for sending to DFM. -> result: success/failure of enqueueing.
    #[allow(async_fn_in_trait)]
    pub fn publish(&self, record: &FaultRecord) -> Result<(), crate::sink::SinkError> {
        // 1) Local log.
        self.logger.on_report(record);

        // 2) Ship the record; the sink decides buffering/retry policy.
        self.sink.publish(record)
    }
}

/// What callers hold and clone in their components.
#[derive(Clone)]
// Reporter carries the static config for a particular component or ECU.
pub struct Reporter {
    cfg: ReporterConfig,
    catalog: Arc<FaultCatalog>,
}

impl Reporter {
    pub fn new(cfg: ReporterConfig, catalog: Arc<FaultCatalog>) -> Self {
        Self { cfg, catalog }
    }
    pub fn cfg(&self) -> &ReporterConfig {
        &self.cfg
    }

    pub fn catalog(&self) -> &FaultCatalog {
        &self.catalog
    }
}
