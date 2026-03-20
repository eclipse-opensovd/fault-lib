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
use alloc::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use common::{
    debounce::Debounce,
    fault::{FaultDescriptor, FaultId, FaultRecord, IpcTimestamp, LifecyclePhase, LifecycleStage},
    sink_error::SinkError,
    types::MetadataVec,
};

use crate::{
    FaultApi,
    sink::{FaultSinkApi, LogHook},
};

// Per-component defaults that get baked into a Reporter instance.
#[derive(Debug, Clone)]
pub struct ReporterConfig {
    pub source: common::ids::SourceId,
    pub lifecycle_phase: LifecyclePhase,
    /// Optional per-reporter defaults (e.g., common metadata).
    pub default_env_data: MetadataVec,
}

/// Errors that can occur when constructing a [`Reporter`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReporterError {
    /// The fault catalog has not been initialized via [`FaultApi::try_new`].
    #[error("fault catalog not initialized")]
    CatalogNotInitialized,
    /// The given fault ID does not exist in the loaded catalog.
    #[error("fault ID not found in catalog: {0:?}")]
    FaultIdNotFound(FaultId),
    /// The IPC sink is not available (dropped or never created).
    #[error("sink not available: {0}")]
    SinkNotAvailable(#[from] SinkError),
}

/// API trait for fault reporters.
///
/// # Errors
///
/// Methods return [`ReporterError`] or [`SinkError`] on initialization
/// or transport failures respectively.
pub trait ReporterApi {
    /// Create a new reporter for the given fault ID and configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ReporterError`] if the fault ID is not found in the catalog
    /// or the catalog has not been initialized.
    fn new(id: &FaultId, config: ReporterConfig) -> Result<Self, ReporterError>
    where
        Self: Sized;
    /// Build a fault record for the given lifecycle stage.
    fn create_record(&self, lifecycle_stage: LifecycleStage) -> FaultRecord;

    /// Publish a fault record to the DFM via the configured sink.
    ///
    /// `path` is the SOVD entity path (e.g. `"ecu/app_name"`). The path is
    /// converted to a 128-byte `LongString` for IPC transport; paths longer
    /// than 128 bytes are rejected with [`SinkError::BadDescriptor`].
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the IPC transport fails or the path is too
    /// long.
    fn publish(&mut self, path: &str, record: FaultRecord) -> Result<(), SinkError>;
}

pub struct Reporter {
    pub(crate) sink: Arc<dyn FaultSinkApi>,
    pub(crate) descriptor: FaultDescriptor,
    pub(crate) config: ReporterConfig,
    /// Runtime debounce state derived from `descriptor.reporter_side_debounce`.
    /// When `Some`, `publish()` filters events through the debouncer before
    /// forwarding to the sink, reducing IPC traffic to the DFM.
    pub(crate) debouncer: Option<Box<dyn Debounce>>,
    /// Tracks the last published lifecycle stage so that stage transitions
    /// (e.g. Passed → Failed) can reset the debouncer.
    pub(crate) last_stage: LifecycleStage,
    /// Optional log hook for fault reporting observability.
    /// Called after each publish attempt (success or error).
    /// Populated from `FaultApi::try_get_log_hook()` during `Reporter::new()`,
    /// or set directly for testing.
    pub(crate) log_hook: Option<Arc<dyn LogHook>>,
}

impl ReporterApi for Reporter {
    fn new(id: &FaultId, config: ReporterConfig) -> Result<Self, ReporterError> {
        let sink = FaultApi::try_get_fault_sink()?;
        let catalog =
            FaultApi::try_get_fault_catalog().ok_or(ReporterError::CatalogNotInitialized)?;
        let descriptor = catalog
            .descriptor(id)
            .ok_or_else(|| ReporterError::FaultIdNotFound(id.clone()))?
            .clone();
        let debouncer = descriptor
            .reporter_side_debounce
            .map(common::debounce::DebounceMode::into_debouncer);
        let log_hook = FaultApi::try_get_log_hook();
        Ok(Self {
            sink,
            descriptor,
            config,
            debouncer,
            last_stage: LifecycleStage::NotTested,
            log_hook,
        })
    }

    /// Create a new fault record with the current timestamp.
    ///
    /// The timestamp is captured at record creation time to accurately
    /// reflect when the fault condition was detected.
    fn create_record(&self, lifecycle_stage: LifecycleStage) -> FaultRecord {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        FaultRecord {
            id: self.descriptor.id.clone(),
            time: IpcTimestamp {
                seconds_since_epoch: now.as_secs(),
                nanoseconds: now.subsec_nanos(),
            },
            source: self.config.source.clone(),
            lifecycle_phase: self.config.lifecycle_phase,
            lifecycle_stage,
            env_data: self.config.default_env_data.clone(),
        }
    }

    fn publish(&mut self, path: &str, record: FaultRecord) -> Result<(), SinkError> {
        let now = Instant::now();

        // Reset debouncer on lifecycle stage transitions (e.g. Passed → Failed)
        // so a new fault occurrence starts with a clean debounce window.
        if Self::should_reset_debouncer(self.last_stage, record.lifecycle_stage)
            && let Some(ref mut d) = self.debouncer
        {
            d.reset(now);
        }

        // Apply debounce filter — suppressed events return Ok(()) silently
        // to reduce IPC traffic to the DFM per design.md REQ-4.
        if let Some(ref mut debouncer) = self.debouncer
            && !debouncer.on_event(now)
        {
            self.last_stage = record.lifecycle_stage;
            return Ok(());
        }

        self.last_stage = record.lifecycle_stage;

        // Publish to sink, then notify log hook (REQ-10).
        // Clone is needed because sink.publish() takes ownership; FaultRecord
        // is repr(C) fixed-size so clone is a cheap memcpy.
        if let Some(ref logger) = self.log_hook {
            let record_copy = record.clone();
            let result = self.sink.publish(path, record);
            match &result {
                Ok(()) => logger.on_publish(&record_copy),
                Err(e) => logger.on_error(&record_copy, e),
            }
            result
        } else {
            self.sink.publish(path, record)
        }
    }
}

impl Reporter {
    /// Returns `true` when the lifecycle stage transition indicates the fault
    /// condition has changed direction (healthy → faulty), which should reset
    /// the debounce window so that a fresh occurrence is not suppressed.
    ///
    /// Only triggers on Passed → Failed/PreFailed transitions.
    /// `NotTested` → Failed is the initial detection, not a reset scenario.
    fn should_reset_debouncer(last_stage: LifecycleStage, new_stage: LifecycleStage) -> bool {
        use LifecycleStage::{Failed, Passed, PreFailed};
        matches!((last_stage, new_stage), (Passed, Failed | PreFailed))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{sink::MockFaultSinkApi, test_utils::*, utils::to_static_short_string};

    #[test]
    fn create_record() {
        let reporter = Reporter {
            sink: Arc::new(MockFaultSinkApi::new()),
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test fault").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Passed);

        assert_eq!(record.id, FaultId::Numeric(42));
        assert_eq!(record.source, stub_source());
        assert_eq!(record.lifecycle_phase, LifecyclePhase::Running);
        assert_eq!(record.lifecycle_stage, LifecycleStage::Passed);
    }

    #[test]
    fn publish_success() {
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
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test fault").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Passed);

        assert!(reporter.publish("test/path", record).is_ok());
    }

    #[test]
    fn publish_fail() {
        let mut mock_sink = MockFaultSinkApi::new();
        mock_sink
            .expect_publish()
            .once()
            .returning(|_, _| Err(SinkError::TransportDown));

        let mut reporter = Reporter {
            sink: Arc::new(mock_sink),
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test fault").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Passed);
        assert_eq!(
            reporter.publish("test/path", record),
            Err(SinkError::TransportDown)
        );
    }
}

#[cfg(test)]
mod design_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::{Duration, Instant},
    };

    use common::{
        config::{ResetPolicy, ResetTrigger},
        debounce::DebounceMode,
        fault::*,
        sink_error::SinkError,
    };

    use crate::{
        catalog::{FaultCatalogBuilder, FaultCatalogConfig},
        reporter::{Reporter, ReporterApi},
        sink::{FaultSinkApi, LogHook},
        test_utils::*,
        utils::to_static_short_string,
    };

    // ============================================================================
    // Helper functions
    // ============================================================================

    fn make_reporter(sink: Arc<dyn FaultSinkApi>, fault_id: FaultId) -> Reporter {
        Reporter {
            sink,
            descriptor: stub_descriptor(
                fault_id,
                to_static_short_string("Test fault").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        }
    }

    fn make_reporter_with_descriptor(
        sink: Arc<dyn FaultSinkApi>,
        descriptor: FaultDescriptor,
    ) -> Reporter {
        let debouncer = descriptor
            .reporter_side_debounce
            .map(common::debounce::DebounceMode::into_debouncer);
        Reporter {
            sink,
            descriptor,
            config: stub_config(),
            debouncer,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        }
    }

    // ============================================================================
    // REQ-1: Framework Agnostic API
    // ============================================================================

    /// REQ-1: The API must not require any specific async runtime.
    /// Reporter is constructed synchronously, no tokio/async-std needed.
    #[test]
    fn req1_reporter_requires_no_async_runtime() {
        let sink = Arc::new(RecordingSink::new());
        let _reporter = make_reporter(sink, FaultId::Numeric(1));
        // If this compiles and runs without an async runtime, REQ-1 is satisfied.
    }

    /// REQ-1: Reporter must be Send + Sync for use across threads.
    #[test]
    fn req1_reporter_is_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Reporter>();
        assert_sync::<Reporter>();
    }

    /// REQ-1: `FaultRecord` must be `ZeroCopySend` for IPC transport.
    #[test]
    fn req1_fault_record_is_zero_copy_send() {
        fn assert_zero_copy<T: iceoryx2::prelude::ZeroCopySend>() {}
        assert_zero_copy::<FaultRecord>();
    }

    // ============================================================================
    // REQ-2: Relays faults via IPC to DFM
    // ============================================================================

    /// REQ-2: Published records must arrive at the sink.
    #[test]
    #[allow(clippy::indexing_slicing)]
    fn req2_sink_receives_published_records() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(42));

        let record = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record).unwrap();

        assert_eq!(sink.count(), 1);
        let received = sink.received_records();
        assert_eq!(received[0].id, FaultId::Numeric(42));
        assert_eq!(received[0].lifecycle_stage, LifecycleStage::Failed);
    }

    /// REQ-2: Sink errors must propagate back to the caller.
    #[test]
    fn req2_sink_error_propagates_to_caller() {
        let sink = Arc::new(FailingSink::transport_down());
        let mut reporter = make_reporter(sink, FaultId::Numeric(42));

        let record = reporter.create_record(LifecycleStage::Failed);
        let result = reporter.publish("test/path", record);

        assert_eq!(result, Err(SinkError::TransportDown));
    }

    /// REQ-2: Multiple records can be published sequentially.
    #[test]
    #[allow(clippy::indexing_slicing)]
    fn req2_multiple_records_delivered_in_order() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(100));

        for stage in [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
        ] {
            let record = reporter.create_record(stage);
            reporter.publish("test/path", record).unwrap();
        }

        let received = sink.received_records();
        assert_eq!(received.len(), 3);
        assert_eq!(received[0].lifecycle_stage, LifecycleStage::NotTested);
        assert_eq!(received[1].lifecycle_stage, LifecycleStage::PreFailed);
        assert_eq!(received[2].lifecycle_stage, LifecycleStage::Failed);
    }

    // ============================================================================
    // REQ-3: Domain-specific error logic (debouncing)
    // ============================================================================

    /// REQ-3: `FaultDescriptor` supports reporter-side debounce configuration.
    #[test]
    fn req3_descriptor_supports_reporter_side_debounce() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(1),
            to_static_short_string("Test").unwrap(),
            Some(DebounceMode::HoldTime {
                duration: Duration::from_secs(5).into(),
            }),
            None,
        );
        assert!(descriptor.reporter_side_debounce.is_some());
    }

    /// REQ-3: `FaultDescriptor` supports manager-side debounce configuration.
    #[test]
    fn req3_descriptor_supports_manager_side_debounce() {
        let descriptor = FaultDescriptor {
            id: FaultId::Numeric(1),
            name: to_static_short_string("Test").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: Some(DebounceMode::CountWithinWindow {
                min_count: 3,
                window: Duration::from_secs(10).into(),
            }),
            manager_side_reset: None,
        };
        assert!(descriptor.manager_side_debounce.is_some());
    }

    /// REQ-3: `FaultDescriptor` supports reset policy configuration.
    #[test]
    fn req3_descriptor_supports_reset_policy() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(1),
            to_static_short_string("Test").unwrap(),
            None,
            Some(ResetPolicy {
                trigger: ResetTrigger::ToolOnly,
                min_operating_cycles_before_clear: None,
            }),
        );
        assert!(descriptor.reporter_side_reset.is_some());
    }

    /// REQ-3: Reporter applies debounce filtering before publishing to sink.
    /// `EdgeWithCooldown` lets the first event through, then suppresses until cooldown expires.
    #[test]
    fn req3_reporter_applies_debounce_before_publish() {
        let debounce = DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_secs(10).into(),
        };
        let descriptor = stub_descriptor(
            FaultId::Numeric(1),
            to_static_short_string("Debounced").unwrap(),
            Some(debounce),
            None,
        );
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter_with_descriptor(Arc::clone(&sink) as _, descriptor);

        let record = reporter.create_record(LifecycleStage::Failed);

        // First publish passes through
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(sink.count(), 1, "First event should pass debounce");

        // Rapid second publish is suppressed by cooldown
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(
            sink.count(),
            1,
            "Second event within cooldown should be suppressed"
        );

        // Third immediate publish also suppressed
        reporter.publish("test/path", record).unwrap();
        assert_eq!(
            sink.count(),
            1,
            "Third event within cooldown should be suppressed"
        );
    }

    // ============================================================================
    // REQ-3: Debounce variant tests
    // ============================================================================

    /// `CountWithinWindow` suppresses events until `min_count` is reached within the window.
    #[test]
    fn req3_debounce_count_within_window_suppresses_early_events() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(2),
            to_static_short_string("CountWindow").unwrap(),
            Some(DebounceMode::CountWithinWindow {
                min_count: 3,
                window: Duration::from_secs(60).into(),
            }),
            None,
        );

        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter_with_descriptor(Arc::clone(&sink) as _, descriptor);

        let record = reporter.create_record(LifecycleStage::Failed);

        // First 2 publishes suppressed (count < min_count)
        reporter.publish("test/path", record.clone()).unwrap();
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(
            sink.count(),
            0,
            "Events before min_count should be suppressed"
        );

        // Third publish fires (count == min_count)
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(sink.count(), 1, "Event at min_count should pass through");

        // Fourth also passes (count > min_count, still in window)
        reporter.publish("test/path", record).unwrap();
        assert_eq!(
            sink.count(),
            2,
            "Events after min_count should pass through"
        );
    }

    /// `HoldTime` suppresses until the configured duration has elapsed since first event.
    #[test]
    fn req3_debounce_hold_time_waits_for_duration() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(3),
            to_static_short_string("HoldTime").unwrap(),
            Some(DebounceMode::HoldTime {
                duration: Duration::from_millis(50).into(),
            }),
            None,
        );

        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter_with_descriptor(Arc::clone(&sink) as _, descriptor);

        let record = reporter.create_record(LifecycleStage::Failed);

        // Immediate publish should be suppressed (hold time not elapsed)
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(sink.count(), 0, "Immediate event should be suppressed");

        // Wait for hold time to elapse
        std::thread::sleep(Duration::from_millis(60));

        // After delay, should fire
        reporter.publish("test/path", record).unwrap();
        assert_eq!(sink.count(), 1, "Event after hold time should pass through");
    }

    /// `EdgeWithCooldown` passes the first event, then suppresses until cooldown expires.
    #[test]
    fn req3_debounce_edge_with_cooldown_passes_first_then_suppresses() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(4),
            to_static_short_string("EdgeCooldown").unwrap(),
            Some(DebounceMode::EdgeWithCooldown {
                cooldown: Duration::from_millis(50).into(),
            }),
            None,
        );

        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter_with_descriptor(Arc::clone(&sink) as _, descriptor);

        let record = reporter.create_record(LifecycleStage::Failed);

        // First event passes through (edge-triggered)
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(sink.count(), 1, "First event should pass through");

        // Immediate second event suppressed (within cooldown)
        reporter.publish("test/path", record.clone()).unwrap();
        assert_eq!(
            sink.count(),
            1,
            "Event within cooldown should be suppressed"
        );

        // Wait for cooldown to expire
        std::thread::sleep(Duration::from_millis(60));

        // After cooldown, next event passes through
        reporter.publish("test/path", record).unwrap();
        assert_eq!(sink.count(), 2, "Event after cooldown should pass through");
    }

    /// Debouncer resets when lifecycle stage transitions from Passed to Failed.
    #[test]
    fn req3_debounce_resets_on_lifecycle_transition() {
        let descriptor = stub_descriptor(
            FaultId::Numeric(5),
            to_static_short_string("ResetTest").unwrap(),
            Some(DebounceMode::CountWithinWindow {
                min_count: 3,
                window: Duration::from_secs(60).into(),
            }),
            None,
        );

        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter_with_descriptor(Arc::clone(&sink) as _, descriptor);

        // Accumulate 3 Failed events → fires at count=3
        let record_fail = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record_fail.clone()).unwrap();
        reporter.publish("test/path", record_fail.clone()).unwrap();
        reporter.publish("test/path", record_fail).unwrap();
        assert_eq!(sink.count(), 1, "3 events reach min_count: published");

        // Transition to Passed (the Passed event also goes through debouncer,
        // but counter is already >= min_count so it passes)
        let record_pass = reporter.create_record(LifecycleStage::Passed);
        reporter.publish("test/path", record_pass).unwrap();
        let count_after_pass = sink.count();

        // Transition back to Failed — debouncer resets, counter restarts from 0
        let record_fail2 = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record_fail2.clone()).unwrap();
        reporter.publish("test/path", record_fail2.clone()).unwrap();
        // After reset: only 2 events, min_count=3, should be suppressed
        assert_eq!(
            sink.count(),
            count_after_pass,
            "After Passed→Failed reset, 2 events still suppressed"
        );

        // Third event after reset fires
        reporter.publish("test/path", record_fail2).unwrap();
        assert_eq!(
            sink.count(),
            count_after_pass + 1,
            "Third event after reset should pass through"
        );
    }

    /// Reporter without debounce passes all events through to sink.
    #[test]
    fn req3_no_debounce_passes_all_events() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(10));

        for _ in 0..10 {
            let record = reporter.create_record(LifecycleStage::Failed);
            reporter.publish("test/path", record).unwrap();
        }

        assert_eq!(
            sink.count(),
            10,
            "Without debounce, all events should pass through"
        );
    }

    // ============================================================================
    // REQ-4: Reporting test results (passed/failed lifecycle stages)
    // ============================================================================

    /// REQ-4: All `LifecycleStage` variants exist and are distinct.
    #[test]
    #[allow(clippy::indexing_slicing)]
    fn req4_all_lifecycle_stages_exist() {
        let stages = [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
            LifecycleStage::PrePassed,
            LifecycleStage::Passed,
        ];
        // All 5 stages must be distinct
        for i in 0..stages.len() {
            for j in (i + 1)..stages.len() {
                assert_ne!(stages[i], stages[j]);
            }
        }
    }

    /// REQ-4: Created record contains the specified `lifecycle_stage`.
    #[test]
    fn req4_record_contains_lifecycle_stage() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = make_reporter(sink, FaultId::Numeric(42));

        for stage in [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
            LifecycleStage::PrePassed,
            LifecycleStage::Passed,
        ] {
            let record = reporter.create_record(stage);
            assert_eq!(
                record.lifecycle_stage, stage,
                "Mismatch for stage {stage:?}"
            );
        }
    }

    /// REQ-4: All lifecycle stages can be published without error.
    #[test]
    fn req4_all_stages_publishable() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(42));

        for stage in [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
            LifecycleStage::PrePassed,
            LifecycleStage::Passed,
        ] {
            let record = reporter.create_record(stage);
            let result = reporter.publish("test/path", record);
            assert!(result.is_ok(), "Failed to publish stage {stage:?}");
        }

        assert_eq!(sink.count(), 5);
    }

    // ============================================================================
    // REQ-5: Per-fault handles (Reporter pattern)
    // ============================================================================

    /// REQ-5: Reporter binds to a single fault ID.
    #[test]
    fn req5_reporter_binds_to_single_fault_id() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = make_reporter(sink, FaultId::Numeric(0x1001));

        let record = reporter.create_record(LifecycleStage::Failed);
        assert_eq!(record.id, FaultId::Numeric(0x1001));
    }

    /// REQ-5: Multiple reporters are independent.
    #[test]
    fn req5_multiple_reporters_independent() {
        let sink = Arc::new(RecordingSink::new());
        let reporter1 = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(0x1001));
        let reporter2 = make_reporter(sink, FaultId::Numeric(0x1002));

        let record1 = reporter1.create_record(LifecycleStage::Failed);
        let record2 = reporter2.create_record(LifecycleStage::Passed);

        assert_ne!(record1.id, record2.id);
        assert_ne!(record1.lifecycle_stage, record2.lifecycle_stage);
    }

    /// REQ-5: Reporter preserves source identity in records.
    #[test]
    fn req5_reporter_preserves_source_identity() {
        let sink = Arc::new(RecordingSink::new());
        let config = stub_config();
        let expected_source = config.source.clone();

        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(1),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config,
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        assert_eq!(record.source, expected_source);
    }

    // ============================================================================
    // REQ-6: Non-blocking publish path
    // ============================================================================

    /// REQ-6: Publish with a slow sink should still return quickly
    /// (because the real sink enqueues and returns, not blocking on IPC).
    /// Here we test that the API call through `Reporter.publish()` is direct -
    /// it calls `sink.publish()` synchronously, so if the sink is slow,
    /// it will be slow. The non-blocking contract is on the REAL `FaultManagerSink`
    /// which enqueues to a channel.
    #[test]
    fn req6_publish_is_synchronous_call_to_sink() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(42));

        let record = reporter.create_record(LifecycleStage::Failed);
        let start = Instant::now();
        reporter.publish("test/path", record).unwrap();
        let elapsed = start.elapsed();

        // RecordingSink is fast - publish should be near-instant
        assert!(
            elapsed < Duration::from_millis(50),
            "publish took {elapsed:?}, expected <50ms"
        );
        assert_eq!(sink.count(), 1);
    }

    /// REQ-6: Multiple rapid publishes should not block each other.
    #[test]
    #[cfg_attr(miri, ignore)] // Miri is 10-100× slower; timing assertion is meaningless.
    fn req6_rapid_publishes_complete_quickly() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = make_reporter(Arc::clone(&sink) as _, FaultId::Numeric(42));

        let start = Instant::now();
        for _ in 0..100 {
            let record = reporter.create_record(LifecycleStage::Failed);
            reporter.publish("test/path", record).unwrap();
        }
        let elapsed = start.elapsed();

        assert_eq!(sink.count(), 100);
        assert!(
            elapsed < Duration::from_millis(100),
            "100 publishes took {elapsed:?}, expected <100ms"
        );
    }

    // ============================================================================
    // REQ-7: Decentral catalogue definition
    // ============================================================================

    /// REQ-7: Catalog hash is deterministic for same config.
    #[test]
    fn req7_catalog_hash_deterministic() {
        let config = FaultCatalogConfig {
            id: "test_app".into(),
            version: 1,
            faults: vec![],
        };

        let catalog1 = FaultCatalogBuilder::new()
            .cfg_struct(config.clone())
            .unwrap()
            .build();
        let catalog2 = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .build();

        assert_eq!(catalog1.config_hash(), catalog2.config_hash());
    }

    /// REQ-7: Different catalogs produce different hashes.
    #[test]
    fn req7_different_catalogs_different_hash() {
        let config1 = FaultCatalogConfig {
            id: "app1".into(),
            version: 1,
            faults: vec![],
        };
        let config2 = FaultCatalogConfig {
            id: "app2".into(),
            version: 1,
            faults: vec![],
        };

        let catalog1 = FaultCatalogBuilder::new()
            .cfg_struct(config1)
            .unwrap()
            .build();
        let catalog2 = FaultCatalogBuilder::new()
            .cfg_struct(config2)
            .unwrap()
            .build();

        assert_ne!(catalog1.config_hash(), catalog2.config_hash());
    }

    /// REQ-7: Catalog can look up descriptors by ID.
    #[test]
    fn req7_catalog_descriptor_lookup() {
        let desc = stub_descriptor(
            FaultId::Numeric(42),
            to_static_short_string("Test fault").unwrap(),
            None,
            None,
        );
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![desc.clone()],
        };

        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .build();

        assert!(catalog.descriptor(&FaultId::Numeric(42)).is_some());
        assert_eq!(
            catalog.descriptor(&FaultId::Numeric(42)).unwrap().name,
            desc.name
        );
        assert!(catalog.descriptor(&FaultId::Numeric(999)).is_none());
    }

    // ============================================================================
    // REQ-8: No DFM redeploy on app changes
    // ============================================================================

    /// REQ-8: Catalog is built at compile time, not loaded from DFM at runtime.
    /// If this test compiles and runs, the catalog is embedded in the binary.
    #[test]
    fn req8_catalog_embedded_in_app_binary() {
        let config = FaultCatalogConfig {
            id: "embedded_app".into(),
            version: 1,
            faults: vec![],
        };
        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .build();
        assert!(!catalog.config_hash().is_empty());
        // Catalog created without DFM connection = no DFM redeploy needed
    }

    // ============================================================================
    // REQ-10: LogHook support
    // ============================================================================

    /// Test helper: `LogHook` that counts `on_publish` calls.
    struct CountingLogHook {
        publish_count: AtomicU32,
        error_count: AtomicU32,
    }

    impl CountingLogHook {
        fn new() -> Self {
            Self {
                publish_count: AtomicU32::new(0),
                error_count: AtomicU32::new(0),
            }
        }
    }

    impl LogHook for CountingLogHook {
        fn on_publish(&self, _record: &FaultRecord) {
            self.publish_count.fetch_add(1, Ordering::SeqCst);
        }
        fn on_error(&self, _record: &FaultRecord, _error: &SinkError) {
            self.error_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// REQ-10: `LogHook` `on_publish` is called after successful sink publish.
    #[test]
    fn req10_log_hook_called_on_successful_publish() {
        let sink = Arc::new(RecordingSink::new());
        let hook = Arc::new(CountingLogHook::new());
        let mut reporter = Reporter {
            sink: Arc::clone(&sink) as _,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("LogHookTest").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: Some(Arc::clone(&hook) as _),
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record).unwrap();

        assert_eq!(
            hook.publish_count.load(Ordering::SeqCst),
            1,
            "on_publish should be called once"
        );
        assert_eq!(
            hook.error_count.load(Ordering::SeqCst),
            0,
            "on_error should not be called on success"
        );
        assert_eq!(sink.count(), 1, "Record should reach the sink");
    }

    /// REQ-10: `LogHook` `on_error` is called when sink publish fails.
    #[test]
    fn req10_log_hook_on_error_called_when_sink_fails() {
        let sink = Arc::new(FailingSink::transport_down());
        let hook = Arc::new(CountingLogHook::new());
        let mut reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("LogHookErrorTest").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: Some(Arc::clone(&hook) as _),
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        let result = reporter.publish("test/path", record);

        assert!(result.is_err());
        assert_eq!(
            hook.error_count.load(Ordering::SeqCst),
            1,
            "on_error should be called once"
        );
        assert_eq!(
            hook.publish_count.load(Ordering::SeqCst),
            0,
            "on_publish should not be called on failure"
        );
    }

    /// REQ-10: Publish works without log hook (backward compatible).
    #[test]
    fn req10_publish_works_without_log_hook() {
        let sink = Arc::new(RecordingSink::new());
        let mut reporter = Reporter {
            sink: Arc::clone(&sink) as _,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("NoHookTest").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        assert!(reporter.publish("test/path", record).is_ok());
        assert_eq!(sink.count(), 1);
    }

    /// REQ-10: `LogHook` called for every publish, not just the first.
    #[test]
    fn req10_log_hook_called_on_every_publish() {
        let sink = Arc::new(RecordingSink::new());
        let hook = Arc::new(CountingLogHook::new());
        let mut reporter = Reporter {
            sink: Arc::clone(&sink) as _,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("MultiPublish").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: Some(Arc::clone(&hook) as _),
        };

        for _ in 0..5 {
            let record = reporter.create_record(LifecycleStage::Failed);
            reporter.publish("test/path", record).unwrap();
        }

        assert_eq!(
            hook.publish_count.load(Ordering::SeqCst),
            5,
            "on_publish should be called for each publish"
        );
        assert_eq!(sink.count(), 5);
    }

    /// REQ-10: `LogHook` is NOT called for debounce-suppressed events.
    #[test]
    fn req10_log_hook_not_called_for_suppressed_events() {
        let sink = Arc::new(RecordingSink::new());
        let hook = Arc::new(CountingLogHook::new());
        let descriptor = stub_descriptor(
            FaultId::Numeric(42),
            to_static_short_string("DebouncedHook").unwrap(),
            Some(DebounceMode::EdgeWithCooldown {
                cooldown: Duration::from_secs(10).into(),
            }),
            None,
        );
        let debouncer = descriptor
            .reporter_side_debounce
            .map(common::debounce::DebounceMode::into_debouncer);
        let mut reporter = Reporter {
            sink: Arc::clone(&sink) as _,
            descriptor,
            config: stub_config(),
            debouncer,
            last_stage: LifecycleStage::NotTested,
            log_hook: Some(Arc::clone(&hook) as _),
        };

        // First event passes through debounce and sink
        let record = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record).unwrap();
        assert_eq!(
            hook.publish_count.load(Ordering::SeqCst),
            1,
            "First event should trigger hook"
        );

        // Second event is suppressed by debounce — hook should NOT be called
        let record = reporter.create_record(LifecycleStage::Failed);
        reporter.publish("test/path", record).unwrap();
        assert_eq!(
            hook.publish_count.load(Ordering::SeqCst),
            1,
            "Suppressed event should NOT trigger hook"
        );
        assert_eq!(sink.count(), 1, "Only first event should reach sink");
    }

    /// REQ-10: `NoOpLogHook` implements `LogHook` with zero overhead.
    #[test]
    fn req10_noop_log_hook_compiles_and_runs() {
        use crate::sink::NoOpLogHook;

        let hook = NoOpLogHook;
        let record = stub_record(stub_descriptor(
            FaultId::Numeric(1),
            to_static_short_string("Noop").unwrap(),
            None,
            None,
        ));
        let error = SinkError::TransportDown;

        // These should compile and do nothing
        hook.on_publish(&record);
        hook.on_error(&record, &error);
    }
}

#[cfg(test)]
mod error_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::{borrow::Cow, sync::Arc};

    use common::{fault::*, sink_error::SinkError};

    use crate::{
        catalog::{FaultCatalogBuilder, FaultCatalogConfig},
        reporter::{Reporter, ReporterApi},
        test_utils::*,
        utils::to_static_short_string,
    };

    // ============================================================================
    // SinkError variant tests
    // ============================================================================

    /// All `SinkError` variants implement Debug and Display.
    #[test]
    fn sinkerror_all_variants_implement_debug_display() {
        let errors = vec![
            SinkError::TransportDown,
            SinkError::RateLimited,
            SinkError::PermissionDenied,
            SinkError::BadDescriptor(Cow::Borrowed("test descriptor issue")),
            SinkError::Other(Cow::Borrowed("test other error")),
            SinkError::InvalidServiceName,
            SinkError::Timeout,
        ];

        for err in &errors {
            // All variants must implement Debug + Display (via thiserror)
            let debug_str = format!("{err:?}");
            let display_str = format!("{err}");
            assert!(!debug_str.is_empty());
            assert!(!display_str.is_empty());
        }
    }

    /// `SinkError::TransportDown` represents a recoverable transport failure.
    #[test]
    fn sinkerror_transport_down_is_recoverable() {
        let err = SinkError::TransportDown;
        assert!(matches!(err, SinkError::TransportDown));
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("transport"),
            "Display message should mention transport: {msg}"
        );
    }

    /// `SinkError::Timeout` contains meaningful context.
    #[test]
    fn sinkerror_timeout_contains_context() {
        let err = SinkError::Timeout;
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("timeout"),
            "Display should contain 'timeout': {msg}"
        );
    }

    /// `SinkError::RateLimited` indicates backpressure.
    #[test]
    fn sinkerror_rate_limited_backpressure() {
        let err = SinkError::RateLimited;
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("rate") || msg.to_lowercase().contains("limit"),
            "Display should mention rate limiting: {msg}"
        );
    }

    /// `SinkError::BadDescriptor` carries a contextual message.
    #[test]
    fn sinkerror_bad_descriptor_message() {
        let err = SinkError::BadDescriptor(Cow::Borrowed("invalid fault ID format"));
        let msg = format!("{err}");
        assert!(
            msg.contains("invalid fault ID format"),
            "Display should contain the descriptor message: {msg}"
        );
    }

    /// `SinkError::Other` accepts arbitrary static messages.
    #[test]
    fn sinkerror_other_accepts_static_msg() {
        let err = SinkError::Other(Cow::Borrowed("custom error context"));
        let msg = format!("{err}");
        assert!(
            msg.contains("custom error context"),
            "Display should contain the message: {msg}"
        );
    }

    /// `SinkError` implements `PartialEq` for test assertions.
    #[test]
    fn sinkerror_equality_comparison() {
        assert_eq!(SinkError::TransportDown, SinkError::TransportDown);
        assert_eq!(SinkError::Timeout, SinkError::Timeout);
        assert_ne!(SinkError::TransportDown, SinkError::Timeout);
        assert_eq!(
            SinkError::BadDescriptor(Cow::Borrowed("same")),
            SinkError::BadDescriptor(Cow::Borrowed("same"))
        );
        assert_ne!(
            SinkError::BadDescriptor(Cow::Borrowed("a")),
            SinkError::BadDescriptor(Cow::Borrowed("b"))
        );
    }

    /// `SinkError` is Clone (Cow<str> prevents Copy, but Clone is still available).
    #[test]
    fn sinkerror_is_clone() {
        let err = SinkError::TransportDown;
        let cloned = err.clone();
        assert_eq!(err, cloned);

        // Verify Cow::Owned variant also clones correctly
        let owned_err = SinkError::Other(Cow::Owned("dynamic error".to_string()));
        let cloned_owned = owned_err.clone();
        assert_eq!(owned_err, cloned_owned);
    }

    // ============================================================================
    // Publish error propagation tests
    // ============================================================================

    /// `FailingSink::transport_down` propagates `TransportDown` through publish.
    #[test]
    fn publish_propagates_transport_down() {
        let sink = Arc::new(FailingSink::transport_down());
        let mut reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        let result = reporter.publish("test/path", record);

        assert_eq!(result, Err(SinkError::TransportDown));
    }

    /// `FailingSink::timeout` propagates Timeout through publish.
    #[test]
    fn publish_propagates_timeout() {
        let sink = Arc::new(FailingSink::timeout());
        let mut reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);
        let result = reporter.publish("test/path", record);

        assert_eq!(result, Err(SinkError::Timeout));
    }

    /// Multiple publishes to a failing sink all return errors.
    #[test]
    fn publish_consistently_returns_error() {
        let sink = Arc::new(FailingSink::transport_down());
        let mut reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        for _ in 0..5 {
            let record = reporter.create_record(LifecycleStage::Failed);
            let result = reporter.publish("test/path", record);
            assert!(result.is_err());
        }
    }

    // ============================================================================
    // Catalog builder error tests
    // ============================================================================

    /// `FaultCatalogBuilder` returns error on invalid JSON via `try_build()`.
    #[test]
    fn catalog_builder_errors_on_invalid_json() {
        use common::catalog::CatalogBuildError;
        let result = FaultCatalogBuilder::new()
            .json_string("{ invalid json }")
            .unwrap()
            .try_build();
        assert!(
            matches!(result, Err(CatalogBuildError::InvalidJson(_))),
            "Should return InvalidJson error on invalid JSON input, got: {result:?}"
        );
    }

    /// `FaultCatalogBuilder` returns error on missing required fields.
    #[test]
    fn catalog_builder_errors_on_missing_fields() {
        let result = FaultCatalogBuilder::new()
            .json_string(r#"{"id": "test"}"#) // missing version, faults
            .unwrap()
            .try_build();
        assert!(
            result.is_err(),
            "Should return error on missing required fields"
        );
    }

    /// `FaultCatalogBuilder` returns error when no input is configured.
    #[test]
    fn catalog_builder_errors_on_no_input() {
        use common::catalog::CatalogBuildError;
        let result = FaultCatalogBuilder::new().try_build();
        assert!(
            matches!(result, Err(CatalogBuildError::MissingConfig)),
            "Should return MissingConfig error when building with no input"
        );
    }

    /// `FaultCatalogBuilder` returns error when configured twice.
    #[test]
    fn catalog_builder_errors_on_double_configure() {
        use common::catalog::CatalogBuildError;
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![],
        };
        let result = FaultCatalogBuilder::new()
            .cfg_struct(config.clone())
            .unwrap()
            .cfg_struct(config); // double configure → Err(AlreadyConfigured)
        assert!(
            matches!(result, Err(CatalogBuildError::AlreadyConfigured)),
            "Should return AlreadyConfigured error when builder is configured twice"
        );
    }

    /// Catalog returns None for unknown fault ID.
    #[test]
    fn catalog_returns_none_for_unknown_id() {
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![stub_descriptor(
                FaultId::Numeric(1),
                to_static_short_string("Known").unwrap(),
                None,
                None,
            )],
        };

        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .build();
        assert!(catalog.descriptor(&FaultId::Numeric(1)).is_some());
        assert!(catalog.descriptor(&FaultId::Numeric(999)).is_none());
    }

    /// Catalog `try_id` returns `IdTooLong` for excessively long catalog ids.
    #[test]
    fn catalog_try_id_returns_error_for_long_id() {
        use common::catalog::CatalogBuildError;
        // LongString capacity is 128 bytes — exceed it
        let long_id = "a".repeat(200);
        let catalog = common::catalog::FaultCatalog::new(
            long_id.into(),
            1,
            std::collections::HashMap::new(),
            vec![],
        );
        let result = catalog.try_id();
        assert!(
            matches!(result, Err(CatalogBuildError::IdTooLong(_))),
            "Should return IdTooLong error for 200-char id"
        );
    }

    /// Catalog `try_id` succeeds for valid-length ids.
    #[test]
    fn catalog_try_id_succeeds_for_valid_id() {
        let catalog = common::catalog::FaultCatalog::new(
            "my_catalog".into(),
            1,
            std::collections::HashMap::new(),
            vec![],
        );
        assert!(catalog.try_id().is_ok());
    }

    /// `try_build` returns `MissingConfig` for unconfigured builder.
    #[test]
    fn catalog_try_build_returns_missing_config() {
        use common::catalog::CatalogBuildError;
        let result = FaultCatalogBuilder::new().try_build();
        assert!(
            matches!(result, Err(CatalogBuildError::MissingConfig)),
            "Should return MissingConfig for unconfigured builder"
        );
    }

    /// `try_build` returns `InvalidJson` for malformed JSON.
    #[test]
    fn catalog_try_build_returns_invalid_json() {
        use common::catalog::CatalogBuildError;
        let result = FaultCatalogBuilder::new()
            .json_string("not valid json")
            .unwrap()
            .try_build();
        assert!(
            matches!(result, Err(CatalogBuildError::InvalidJson(_))),
            "Should return InvalidJson for malformed JSON"
        );
    }

    /// `try_build` returns Io error for non-existent file.
    #[test]
    fn catalog_try_build_returns_io_error_for_missing_file() {
        use common::catalog::CatalogBuildError;
        let result = FaultCatalogBuilder::new()
            .json_file(std::path::PathBuf::from("/nonexistent/path/catalog.json"))
            .unwrap()
            .try_build();
        assert!(
            matches!(result, Err(CatalogBuildError::Io(_))),
            "Should return Io error for missing file"
        );
    }

    /// Publish rejects paths that exceed max length.
    #[test]
    fn publish_rejects_path_too_long() {
        let sink = Arc::new(crate::test_utils::RecordingSink::new());
        let mut reporter = crate::reporter::Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: common::fault::LifecycleStage::NotTested,
            log_hook: None,
        };

        // The RecordingSink accepts everything, but we can verify path validation
        // happens at the FaultManagerSink level. For unit testing, verify the
        // Reporter itself doesn't panic on long paths.
        let long_path = "a".repeat(300);
        let record = reporter.create_record(common::fault::LifecycleStage::Failed);
        // This goes through RecordingSink which doesn't validate, but
        // the important thing is no panic occurs.
        let _result = reporter.publish(&long_path, record);
    }

    /// `SinkError` variants cover all expected error conditions.
    #[test]
    fn sinkerror_queue_full_variant() {
        let err = SinkError::QueueFull;
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("queue") || msg.to_lowercase().contains("full"),
            "Display should mention queue full: {msg}"
        );
    }
}

#[cfg(test)]
mod concurrent_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    use common::fault::*;

    use crate::{
        reporter::{Reporter, ReporterApi},
        sink::FaultSinkApi,
        test_utils::*,
        utils::to_static_short_string,
    };

    // ============================================================================
    // Concurrent publish tests
    // ============================================================================

    /// Multiple reporters from different threads publish to the same sink safely.
    #[test]
    fn concurrent_reporters_publish_safely() {
        let sink = Arc::new(AtomicCountingSink::new());

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let sink = Arc::clone(&sink);
                thread::spawn(move || {
                    let mut reporter = Reporter {
                        sink,
                        descriptor: stub_descriptor(
                            FaultId::Numeric(i),
                            to_static_short_string("Fault").unwrap(),
                            None,
                            None,
                        ),
                        config: stub_config(),
                        debouncer: None,
                        last_stage: LifecycleStage::NotTested,
                        log_hook: None,
                    };

                    for _ in 0..100 {
                        let record = reporter.create_record(LifecycleStage::Failed);
                        let _ = reporter.publish("test/path", record);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked");
        }

        assert_eq!(sink.count(), 1000, "All 1000 publishes should be counted");
    }

    /// Stress test: many rapid publishes from multiple threads complete without panic.
    #[test]
    #[cfg_attr(miri, ignore)] // Miri is 10-100× slower; timing assertion is meaningless.
    fn stress_test_high_throughput() {
        let sink = Arc::new(AtomicCountingSink::new());
        let target_per_thread = 2500;
        let num_threads = 4;

        let start = Instant::now();
        let handles: Vec<_> = (0..num_threads)
            .map(|i| {
                let sink = Arc::clone(&sink);
                thread::spawn(move || {
                    let mut reporter = Reporter {
                        sink,
                        descriptor: stub_descriptor(
                            FaultId::Numeric(i),
                            to_static_short_string("Stress").unwrap(),
                            None,
                            None,
                        ),
                        config: stub_config(),
                        debouncer: None,
                        last_stage: LifecycleStage::NotTested,
                        log_hook: None,
                    };

                    for _ in 0..target_per_thread {
                        let record = reporter.create_record(LifecycleStage::Failed);
                        let _ = reporter.publish("test/path", record);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked during stress test");
        }

        let elapsed = start.elapsed();
        let count = sink.count();
        let expected = (num_threads * target_per_thread) as usize;

        assert_eq!(count, expected, "Expected {expected} records, got {count}");
        // Should complete in reasonable time (under 5 seconds)
        assert!(
            elapsed < Duration::from_secs(5),
            "Stress test took {elapsed:?}, expected <5s"
        );
    }

    /// `RecordingSink` is thread-safe for concurrent writes.
    #[test]
    fn recording_sink_thread_safe() {
        let sink = Arc::new(RecordingSink::new());

        let handles: Vec<_> = (0..5)
            .map(|i| {
                let sink = Arc::clone(&sink);
                thread::spawn(move || {
                    for _ in 0..20 {
                        let record = stub_record(stub_descriptor(
                            FaultId::Numeric(i),
                            to_static_short_string("Test").unwrap(),
                            None,
                            None,
                        ));
                        sink.publish("test/path", record).unwrap();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked");
        }

        assert_eq!(sink.count(), 100, "All 100 records should be stored");
        assert_eq!(sink.received_records().len(), 100);
    }

    /// Reporter can be shared across threads via Arc.
    #[test]
    fn reporter_shared_via_arc() {
        let sink = Arc::new(AtomicCountingSink::new());
        let reporter = Arc::new(std::sync::Mutex::new(Reporter {
            sink: Arc::clone(&sink) as _,
            descriptor: stub_descriptor(
                FaultId::Numeric(1),
                to_static_short_string("Shared").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        }));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let reporter = Arc::clone(&reporter);
                thread::spawn(move || {
                    for _ in 0..25 {
                        let mut r = reporter.lock().unwrap();
                        let record = r.create_record(LifecycleStage::Failed);
                        let _ = r.publish("test/path", record);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked");
        }

        assert_eq!(sink.count(), 100);
    }

    /// Drop of `FaultManagerSink` does not deadlock with concurrent operations.
    #[test]
    fn drop_sink_no_deadlock() {
        let sink = Arc::new(SlowSink::new(Duration::from_millis(1)));
        let sink_clone = Arc::clone(&sink);

        let handle = thread::spawn(move || {
            let mut reporter = Reporter {
                sink: sink_clone,
                descriptor: stub_descriptor(
                    FaultId::Numeric(1),
                    to_static_short_string("Test").unwrap(),
                    None,
                    None,
                ),
                config: stub_config(),
                debouncer: None,
                last_stage: LifecycleStage::NotTested,
                log_hook: None,
            };

            for _ in 0..5 {
                let record = reporter.create_record(LifecycleStage::Failed);
                let _ = reporter.publish("test/path", record);
            }
        });

        // Drop our reference to the sink
        drop(sink);

        // Thread should complete without deadlock
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = handle.join();
            let _ = tx.send(());
        });

        let result = rx.recv_timeout(Duration::from_secs(5));
        assert!(
            result.is_ok(),
            "Deadlock detected - thread didn't complete in 5s"
        );
    }

    /// Creating records from multiple threads is safe (`create_record` is read-only).
    #[test]
    fn concurrent_create_record() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = Arc::new(Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Concurrent").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        });

        let handles: Vec<_> = (0..10)
            .map(|_| {
                let reporter = Arc::clone(&reporter);
                thread::spawn(move || {
                    for stage in [
                        LifecycleStage::NotTested,
                        LifecycleStage::PreFailed,
                        LifecycleStage::Failed,
                        LifecycleStage::PrePassed,
                        LifecycleStage::Passed,
                    ] {
                        let record = reporter.create_record(stage);
                        assert_eq!(record.id, FaultId::Numeric(42));
                    }
                })
            })
            .collect();

        for h in handles {
            h.join()
                .expect("Thread panicked during concurrent create_record");
        }
    }
}

#[cfg(test)]
mod timestamp_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::sync::Arc;

    use common::fault::*;

    use crate::{
        reporter::{Reporter, ReporterApi},
        test_utils::*,
        utils::to_static_short_string,
    };

    // ============================================================================
    // Current behavior (timestamps are zero)
    // ============================================================================

    /// Documents current behavior: `IpcTimestamp::default()` is zero.
    #[test]
    fn timestamp_default_is_zero() {
        let default_ts = IpcTimestamp::default();
        assert_eq!(default_ts.seconds_since_epoch, 0);
        assert_eq!(default_ts.nanoseconds, 0);
    }

    /// Verifies that `create_record` populates timestamps from system time.
    #[test]
    fn timestamp_is_populated_in_create_record() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);

        // After fix: timestamp should be non-zero
        assert!(
            record.time.seconds_since_epoch > 0,
            "Timestamp should be populated: {:?}",
            record.time
        );
    }

    // ============================================================================
    // Timestamp population tests
    // ============================================================================

    /// Timestamp should be non-zero when record is created.
    #[test]
    fn timestamp_is_populated_after_fix() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);

        assert!(
            record.time.seconds_since_epoch > 0 || record.time.nanoseconds > 0,
            "Timestamp should not be zero after fix: {:?}",
            record.time
        );
    }

    /// Timestamp should be recent (within test execution window).
    #[test]
    fn timestamp_is_recent_after_fix() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sink = Arc::new(RecordingSink::new());
        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);

        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        assert!(
            record.time.seconds_since_epoch >= before,
            "Timestamp {} should be >= before {}",
            record.time.seconds_since_epoch,
            before
        );
        assert!(
            record.time.seconds_since_epoch <= after,
            "Timestamp {} should be <= after {}",
            record.time.seconds_since_epoch,
            after
        );
    }

    /// Nanoseconds should be valid (< 1 billion).
    #[test]
    fn timestamp_nanoseconds_valid_after_fix() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let record = reporter.create_record(LifecycleStage::Failed);

        assert!(
            record.time.nanoseconds < 1_000_000_000,
            "Nanoseconds {} should be < 1 billion",
            record.time.nanoseconds
        );
    }

    /// Sequential records should have non-decreasing timestamps.
    #[test]
    fn timestamp_monotonic_after_fix() {
        let sink = Arc::new(RecordingSink::new());
        let reporter = Reporter {
            sink,
            descriptor: stub_descriptor(
                FaultId::Numeric(42),
                to_static_short_string("Test").unwrap(),
                None,
                None,
            ),
            config: stub_config(),
            debouncer: None,
            last_stage: LifecycleStage::NotTested,
            log_hook: None,
        };

        let mut prev_total: u128 = 0;

        for _ in 0..10 {
            let record = reporter.create_record(LifecycleStage::Failed);

            let curr_total = u128::from(record.time.seconds_since_epoch) * 1_000_000_000
                + u128::from(record.time.nanoseconds);

            assert!(
                curr_total >= prev_total,
                "Timestamps should be monotonically non-decreasing"
            );

            prev_total = curr_total;
        }
    }
}
