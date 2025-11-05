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
    ids::FaultId,
    model::{FaultDescriptor, FaultLifecycleStage, FaultRecord},
    sink::{FaultSink, LogHook},
};
use std::{sync::Arc, time::SystemTime};

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

/// Per-fault reporter bound to a specific fault descriptor.
/// Create one instance per fault at startup.
#[derive(Clone)]
pub struct Reporter {
    fault_id: FaultId,
    descriptor: FaultDescriptor,
    cfg: ReporterConfig,
    api: Arc<FaultApi>,
}

impl Reporter {
    /// Create a new Reporter bound to a specific fault ID.
    /// This should be called once per fault during initialization.
    pub fn new(
        api: Arc<FaultApi>,
        catalog: &FaultCatalog,
        cfg: ReporterConfig,
        fault_id: &FaultId,
    ) -> Self {
        let descriptor = catalog
            .find(fault_id)
            .expect("fault ID must exist in catalog")
            .clone();

        Self {
            fault_id: fault_id.clone(),
            descriptor,
            cfg,
            api,
        }
    }

    /// Create a new fault record for this specific fault.
    /// The returned record can be mutated before publishing.
    pub fn create_record(&self) -> FaultRecord {
        FaultRecord {
            fault_id: self.fault_id.clone(),
            time: SystemTime::now(),
            severity: self.descriptor.default_severity,
            source: self.cfg.source.clone(),
            lifecycle_phase: self.cfg.lifecycle_phase,
            stage: FaultLifecycleStage::Raised,
            metadata: self.cfg.default_meta.clone(),
        }
    }

    /// Publish a fault record. Always logs via LogHook, then publishes via sink.
    pub fn publish(&self, record: &FaultRecord) -> Result<(), crate::sink::SinkError> {
        debug_assert_eq!(
            &record.fault_id, &self.fault_id,
            "FaultRecord fault_id doesn't match Reporter"
        );
        self.api.publish(record)
    }

    /// Convenience: create and return a record with Active stage
    pub fn raise(&self) -> FaultRecord {
        let mut rec = self.create_record();
        rec.update_stage(FaultLifecycleStage::Active);
        rec
    }

    /// Convenience: create and return a record with Cleared stage
    pub fn clear(&self) -> FaultRecord {
        let mut rec = self.create_record();
        rec.update_stage(FaultLifecycleStage::Cleared);
        rec
    }
}
