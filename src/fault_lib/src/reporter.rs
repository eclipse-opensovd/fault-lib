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
use crate::{FaultApi, sink::*};
use common::{fault::*, sink_error::*, types::*};
use std::sync::Arc;

// Per-component defaults that get baked into a Reporter instance.
#[derive(Debug, Clone)]
pub struct ReporterConfig {
    pub source: common::ids::SourceId,
    pub lifecycle_phase: LifecyclePhase,
    /// Optional per-reporter defaults (e.g., common metadata).
    pub default_env_data: MetadataVec,
}

pub trait ReporterApi {
    fn new(id: &FaultId, config: ReporterConfig) -> Option<Self>
    where
        Self: Sized;
    fn create_record(&self, lifecycle_stage: LifecycleStage) -> FaultRecord;
    fn publish(&mut self, path: &str, record: FaultRecord) -> Result<(), SinkError>;
}

pub struct Reporter {
    sink: Arc<dyn FaultSinkApi>,
    descriptor: FaultDescriptor,
    config: ReporterConfig,
}

impl ReporterApi for Reporter {
    fn new(id: &FaultId, config: ReporterConfig) -> Option<Self> {
        Some(Self {
            sink: FaultApi::get_fault_sink(),
            descriptor: FaultApi::get_fault_catalog().descriptor(id)?.clone(),
            config,
        })
    }

    // TODO: Discuss:
    // - Should severity be modifiable, and on the record?
    // - Why have an API to modifying env_data instead of just allowing the user to modify the vector.
    // - When should time be set?
    fn create_record(&self, lifecycle_stage: LifecycleStage) -> FaultRecord {
        FaultRecord {
            id: self.descriptor.id.clone(),
            time: IpcTimestamp {
                seconds_since_epoch: (0),
                nanoseconds: (0),
            }, // TODO: When should "now" be? Create time? Publish time?
            source: self.config.source.clone(),
            lifecycle_phase: self.config.lifecycle_phase,
            lifecycle_stage,
            env_data: self.config.default_env_data.clone(),
        }
    }

    fn publish(&mut self, path: &str, record: FaultRecord) -> Result<(), SinkError> {
        // Notes on debouncing:
        // - Use the record's lifecycle_stage to determine whether the fault is active or not.
        // - Use the descriptor's reporter_side_debounce to determine debounce policy for the reporter.

        // To be discussed:
        // - What happens to record's source and metadata when the debounce discards the record?

        self.sink.publish(path, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::MockFaultSinkApi;
    use crate::test_utils::*;
    use crate::utils::to_static_short_string;

    #[test]
    fn create_record() {
        let reporter = Reporter {
            sink: Arc::new(MockFaultSinkApi::new()),
            descriptor: stub_descriptor(FaultId::Numeric(42), to_static_short_string("Test fault").unwrap(), None, None),
            config: stub_config(),
        };

        let record = reporter.create_record(LifecycleStage::Passed);

        assert_eq!(record.id, FaultId::Numeric(42));
        assert_eq!(record.source, stub_source());
        assert_eq!(record.lifecycle_phase, LifecyclePhase::Running);
        assert_eq!(record.lifecycle_stage, LifecycleStage::Passed);
    }

    #[test]
    fn publsh_success() {
        let mut mock_sink = MockFaultSinkApi::new();
        mock_sink.expect_publish().once().returning(|path, record| {
            assert_eq!(path, "test/path");
            assert_eq!(record.id, FaultId::Numeric(42));
            assert_eq!(record.source, stub_source());
            assert_eq!(record.lifecycle_phase, LifecyclePhase::Running);
            assert_eq!(record.lifecycle_stage, LifecycleStage::Passed);

            Ok(())
        });

        let mut reporter = Reporter {
            sink: Arc::new(mock_sink),
            descriptor: stub_descriptor(FaultId::Numeric(42), to_static_short_string("Test fault").unwrap(), None, None),
            config: stub_config(),
        };

        let record = reporter.create_record(LifecycleStage::Passed);

        assert!(reporter.publish("test/path", record).is_ok());
    }

    #[test]
    fn publish_fail() {
        let mut mock_sink = MockFaultSinkApi::new();
        mock_sink.expect_publish().once().returning(|_, _| Err(SinkError::TransportDown));

        let mut reporter = Reporter {
            sink: Arc::new(mock_sink),
            descriptor: stub_descriptor(FaultId::Numeric(42), to_static_short_string("Test fault").unwrap(), None, None),
            config: stub_config(),
        };

        let record = reporter.create_record(LifecycleStage::Passed);
        assert_eq!(reporter.publish("test/path", record), Err(SinkError::TransportDown));
    }
}
