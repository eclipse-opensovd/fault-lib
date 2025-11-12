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
    sink::{FaultSink, LogHook, SinkError},
};
use std::{sync::{Arc, OnceLock}, time::SystemTime};

// FaultApi acts as a singleton façade. A component initializes it once and
// subsequent publishing paths retrieve the sink/logger via global accessors.
pub struct FaultApi;

static SINK: OnceLock<Arc<dyn FaultSink>> = OnceLock::new();
static LOGGER: OnceLock<Arc<dyn LogHook>> = OnceLock::new();

impl FaultApi {
    /// Initialize the singleton. Safe to call once; subsequent calls are ignored.
    pub fn new(sink: Arc<dyn FaultSink>, logger: Arc<dyn LogHook>) -> Self {
        let _ = SINK.set(Arc::clone(&sink));
        let _ = LOGGER.set(Arc::clone(&logger));
        FaultApi
    }

    pub(crate) fn get_sink() -> Arc<dyn FaultSink> {
        SINK.get()
            .cloned()
            .expect("Sink not initialized - call FaultApi::new() before creating reporters")
    }

    pub(crate) fn get_logger() -> Arc<dyn LogHook> {
        LOGGER.get()
            .cloned()
            .expect("Logger not initialized - call FaultApi::new() before creating reporters")
    }

    /// Publish a record: log locally then enqueue via sink. Non-blocking semantics depend on sink impl.
    pub fn publish(record: &FaultRecord) -> Result<(), SinkError> {
        FaultApi::get_logger().on_report(record);
        FaultApi::get_sink().publish(record)
    }
}

/// Per-fault reporter bound to a specific fault descriptor.
/// Create one instance per fault at startup.
#[derive(Clone)]
pub struct Reporter {
    fault_id: FaultId,
    descriptor: FaultDescriptor,
    cfg: ReporterConfig,
}

impl Reporter {
    /// Create a new Reporter bound to a specific fault ID.
    /// This should be called once per fault during initialization.
    pub fn new(
        catalog: &FaultCatalog,
        cfg: ReporterConfig,
        fault_id: &FaultId,
    ) -> Self {
        let descriptor = catalog
            .find(fault_id)
            .expect("fault ID must exist in catalog")
            .clone();

        Self { fault_id: fault_id.clone(), descriptor, cfg }
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
            stage: FaultLifecycleStage::NotTested,
            environment_data: self.cfg.default_environment_data.clone(),
        }
    }

    /// Publish a fault record. Always logs via LogHook, then publishes via sink.
    pub fn publish(&self, record: &FaultRecord) -> Result<(), crate::sink::SinkError> {
        debug_assert_eq!(
            &record.fault_id, &self.fault_id,
            "FaultRecord fault_id doesn't match Reporter"
        );
        FaultApi::publish(record)
    }

    /// Convenience: create and return a record with Failed stage (confirmed failure)
    pub fn fail(&self) -> FaultRecord {
        let mut rec = self.create_record();
        rec.update_stage(FaultLifecycleStage::Failed);
        rec
    }

    /// Convenience: create and return a record with Passed stage (healthy)
    pub fn pass(&self) -> FaultRecord {
        let mut rec = self.create_record();
        rec.update_stage(FaultLifecycleStage::Passed);
        rec
    }
}
