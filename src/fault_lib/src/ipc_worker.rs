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
use alloc::{collections::VecDeque, sync::Weak};
use core::time::Duration;
use std::{sync::mpsc, time::Instant};

use common::{
    enabling_condition::EnablingConditionNotification,
    ipc_service_name::{
        DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME, ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME,
    },
    ipc_service_type::ServiceType,
    sink_error::SinkError,
    types::DiagnosticEvent,
};
use iceoryx2::{
    port::{publisher::Publisher, subscriber::Subscriber},
    prelude::{NodeBuilder, ServiceName},
};
use tracing::{debug, error, info, warn};

use crate::{
    enabling_condition::EnablingConditionManager,
    fault_manager_sink::{SinkInitError, WorkerMsg, WorkerReceiver},
};

// ============================================================================
// Retry Configuration (fault_lib-internal, NOT transferred via IPC)
// ============================================================================

/// Configuration for IPC retry behavior within the worker thread.
///
/// This struct is internal to `fault_lib` and controls how the IPC worker
/// handles transient send failures. It is NOT part of the IPC protocol
/// and NOT transferred over iceoryx2.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts per fault before dropping.
    pub max_retries: u32,
    /// Maximum number of faults to cache for retry.
    pub cache_capacity: usize,
    /// Base interval between retry attempts (exponential backoff base).
    pub retry_interval: Duration,
    /// Maximum retry interval (exponential backoff cap).
    pub max_retry_interval: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 10,
            cache_capacity: 512,
            retry_interval: Duration::from_millis(100),
            max_retry_interval: Duration::from_secs(5),
        }
    }
}

// ============================================================================
// Cached Fault Entry
// ============================================================================

/// A diagnostic event that failed to send and is queued for retry.
struct CachedFault {
    event: DiagnosticEvent,
    attempts: u32,
    next_retry: Instant,
}

impl CachedFault {
    fn new(event: DiagnosticEvent) -> Self {
        Self {
            event,
            attempts: 0,
            next_retry: Instant::now(),
        }
    }

    /// Calculate exponential backoff delay: base × 2^attempts, capped at max.
    ///
    /// Progression with default config: 100ms → 200ms → 400ms → 800ms → 1.6s → 3.2s → 5s (cap)
    fn backoff_delay(&self, config: &RetryConfig) -> Duration {
        let multiplier = 2u32.saturating_pow(self.attempts.min(10));
        let delay = config.retry_interval.saturating_mul(multiplier);
        delay.min(config.max_retry_interval)
    }
}

// ============================================================================
// IPC Worker Retry State
// ============================================================================

/// Internal retry queue state for the IPC worker.
///
/// Extracted as a separate struct for testability — the retry logic
/// can be tested without iceoryx2 by providing a mock publish function.
pub(crate) struct IpcWorkerState {
    retry_queue: VecDeque<CachedFault>,
    config: RetryConfig,
}

impl IpcWorkerState {
    fn new(config: RetryConfig) -> Self {
        Self {
            retry_queue: VecDeque::new(),
            config,
        }
    }

    /// Handle a failed send by caching for retry (transient) or logging and
    /// dropping (permanent).
    fn handle_send_failure(&mut self, event: DiagnosticEvent, error: &SinkError) {
        if Self::is_transient(error) {
            self.cache_for_retry(event);
        } else {
            error!("Permanent IPC error, dropping event: {error:?}");
        }
    }

    /// Add a failed event to the retry queue, evicting oldest if at capacity.
    fn cache_for_retry(&mut self, event: DiagnosticEvent) {
        if self.retry_queue.len() >= self.config.cache_capacity
            && let Some(evicted) = self.retry_queue.pop_front()
        {
            warn!(
                "Retry cache full, evicting event after {} attempts",
                evicted.attempts
            );
        }
        self.retry_queue.push_back(CachedFault::new(event));
    }

    /// Process retry queue — called periodically in worker loop.
    ///
    /// The `publish_fn` closure abstracts the actual IPC send, enabling
    /// unit testing without iceoryx2.
    #[allow(clippy::arithmetic_side_effects)]
    fn process_retries<F>(&mut self, publish_fn: &F)
    where
        F: Fn(&DiagnosticEvent) -> Result<(), SinkError>,
    {
        let now = Instant::now();
        let mut still_pending = VecDeque::new();

        while let Some(mut cached) = self.retry_queue.pop_front() {
            if cached.next_retry > now {
                still_pending.push_back(cached);
                continue;
            }

            cached.attempts += 1;

            match publish_fn(&cached.event) {
                Ok(()) => {
                    debug!("Retry success after {} attempts", cached.attempts);
                }
                Err(ref e) if cached.attempts < self.config.max_retries => {
                    let delay = cached.backoff_delay(&self.config);
                    debug!(
                        "Retry {} failed, next retry in {:?}: {:?}",
                        cached.attempts, delay, e
                    );
                    cached.next_retry = now + delay;
                    still_pending.push_back(cached);
                }
                Err(e) => {
                    error!("Dropping event after {} attempts: {:?}", cached.attempts, e);
                }
            }
        }

        self.retry_queue = still_pending;
    }

    /// Determine if an error is transient (worth retrying) or permanent.
    fn is_transient(error: &SinkError) -> bool {
        matches!(
            error,
            SinkError::TransportDown
                | SinkError::Timeout
                | SinkError::QueueFull
                | SinkError::RateLimited
        )
    }

    /// Number of events currently in the retry queue.
    #[cfg(all(test, not(miri)))]
    fn retry_queue_len(&self) -> usize {
        self.retry_queue.len()
    }
}

// ============================================================================
// IPC Worker
// ============================================================================

/// Timeout for `recv_timeout` in the worker loop.
/// Short enough to process retries promptly, long enough to avoid busy-waiting.
const RECV_TIMEOUT: Duration = Duration::from_millis(50);

pub struct IpcWorker {
    sink_receiver: WorkerReceiver,
    diagnostic_publisher: Option<Publisher<ServiceType, DiagnosticEvent, ()>>,
    ec_notification_subscriber: Option<Subscriber<ServiceType, EnablingConditionNotification, ()>>,
    ec_manager: Weak<EnablingConditionManager>,
    state: IpcWorkerState,
}

impl IpcWorker {
    pub fn new(
        sink_receiver: WorkerReceiver,
        ec_manager: Weak<EnablingConditionManager>,
    ) -> Result<Self, SinkInitError> {
        Self::with_retry_config(sink_receiver, RetryConfig::default(), ec_manager)
    }

    /// Create an IPC worker with custom retry config.
    ///
    /// Uses the default production service name for the event publisher.
    pub fn with_retry_config(
        sink_receiver: WorkerReceiver,
        retry_config: RetryConfig,
        ec_manager: Weak<EnablingConditionManager>,
    ) -> Result<Self, SinkInitError> {
        Self::create(
            sink_receiver,
            retry_config,
            DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME,
            ec_manager,
        )
    }

    /// Create an IPC worker with a custom service name (test isolation).
    ///
    /// Each test can supply a unique service name to avoid iceoryx2 shared
    /// memory conflicts when tests run in parallel.
    #[cfg(all(test, not(miri)))]
    pub fn with_test_service(
        sink_receiver: WorkerReceiver,
        retry_config: RetryConfig,
        service_name: &str,
    ) -> Result<Self, SinkInitError> {
        Self::create(
            sink_receiver,
            retry_config,
            service_name,
            Weak::<EnablingConditionManager>::new(),
        )
    }

    /// Internal constructor: creates iceoryx2 node, service, and publisher.
    ///
    /// # Errors
    ///
    /// Returns `SinkInitError::IpcService` if iceoryx2 node, service, or
    /// publisher creation fails (e.g. no shared memory, no permissions).
    fn create(
        sink_receiver: WorkerReceiver,
        retry_config: RetryConfig,
        service_name: &str,
        ec_manager: Weak<EnablingConditionManager>,
    ) -> Result<Self, SinkInitError> {
        let node = NodeBuilder::new()
            .create::<ServiceType>()
            .map_err(|e| SinkInitError::IpcService(format!("node creation: {e}")))?;
        let event_publisher_service_name = ServiceName::new(service_name).map_err(|e| {
            SinkInitError::IpcService(format!("service name '{service_name}': {e}"))
        })?;
        let event_publisher_service = node
            .service_builder(&event_publisher_service_name)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .map_err(|e| SinkInitError::IpcService(format!("event publisher service: {e}")))?;
        let publisher = event_publisher_service
            .publisher_builder()
            .create()
            .map_err(|e| SinkInitError::IpcService(format!("event publisher: {e}")))?;

        // EC notification subscriber is best-effort — may fail if DFM not running
        let ec_notification_subscriber =
            ServiceName::new(ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME)
                .ok()
                .and_then(|svc_name| {
                    node.service_builder(&svc_name)
                        .publish_subscribe::<EnablingConditionNotification>()
                        .open_or_create()
                        .ok()
                })
                .and_then(|service| service.subscriber_builder().create().ok());

        if ec_notification_subscriber.is_some() {
            debug!("EC notification subscriber created");
        } else {
            debug!("EC notification subscriber not available (DFM may not be running)");
        }

        Ok(Self {
            sink_receiver,
            diagnostic_publisher: Some(publisher),
            ec_notification_subscriber,
            ec_manager,
            state: IpcWorkerState::new(retry_config),
        })
    }

    /// Attempt to publish an event via iceoryx2.
    ///
    /// Takes a reference and clones into shared memory — the clone cost is
    /// negligible for `#[repr(C)]` fixed-size types (effectively a memcpy).
    fn publish_event(&self, event: &DiagnosticEvent) -> Result<(), SinkError> {
        let publisher = self
            .diagnostic_publisher
            .as_ref()
            .ok_or(SinkError::TransportDown)?;
        let sample = publisher
            .loan_uninit()
            .map_err(|_| SinkError::TransportDown)?;
        let sample = sample.write_payload(event.clone());
        match sample.send().map_err(|_| SinkError::TransportDown) {
            Ok(_) => {
                debug!("Event successfully sent!");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn run(&mut self) {
        // Handle Start message first (blocking, parent thread is waiting to be unparked)
        if let Ok(WorkerMsg::Start(parent)) = self.sink_receiver.recv() {
            debug!("Diag IPC worker running");
            parent.unpark();
        }

        // Main loop: process channel messages + retry queue + EC notifications
        loop {
            match self.sink_receiver.recv_timeout(RECV_TIMEOUT) {
                Ok(WorkerMsg::Event { event }) => {
                    if let Err(e) = self.publish_event(&event) {
                        // Only retry Fault events; Hash/EC events are time-sensitive
                        if matches!(event.as_ref(), DiagnosticEvent::Fault(_)) {
                            self.state.handle_send_failure(*event, &e);
                        } else {
                            error!("Event send failed (not retrying): {e:?}");
                        }
                    }
                }
                Ok(WorkerMsg::Start(parent)) => {
                    debug!("Late start message received");
                    parent.unpark();
                }
                Ok(WorkerMsg::Exit) => {
                    info!("FaultMgrClient worker ends");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No new messages — fall through to retry + EC notification processing
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    info!("Channel disconnected, worker exiting");
                    break;
                }
            }

            // Process retry queue (borrows publisher and state as separate fields)
            let publisher = &self.diagnostic_publisher;
            self.state.process_retries(&|event: &DiagnosticEvent| {
                let pub_ref = publisher.as_ref().ok_or(SinkError::TransportDown)?;
                let sample = pub_ref
                    .loan_uninit()
                    .map_err(|_| SinkError::TransportDown)?;
                let sample = sample.write_payload(event.clone());
                sample
                    .send()
                    .map_err(|_| SinkError::TransportDown)
                    .map(|_| ())
            });

            // Poll for enabling condition notifications from DFM
            self.poll_ec_notifications();
        }

        // Final flush: attempt to deliver remaining cached faults before shutdown
        self.final_flush();
    }

    /// Poll the enabling condition notification subscriber and dispatch
    /// received notifications to the local `EnablingConditionManager`.
    fn poll_ec_notifications(&self) {
        let Some(subscriber) = self.ec_notification_subscriber.as_ref() else {
            return;
        };

        let Some(manager) = self.ec_manager.upgrade() else {
            return;
        };

        // Drain all available notifications
        loop {
            match subscriber.receive() {
                Ok(Some(sample)) => {
                    let notification = sample.payload();
                    debug!(
                        "Received EC notification: {} -> {:?}",
                        notification.id, notification.status
                    );
                    manager.handle_remote_notification(&notification.id, notification.status);
                }
                Ok(None) => break,
                Err(e) => {
                    error!("EC notification receive error: {e:?}");
                    break;
                }
            }
        }
    }

    /// Attempt to deliver any remaining cached faults before shutdown.
    fn final_flush(&mut self) {
        let remaining = self.state.retry_queue.len();
        if remaining > 0 {
            info!("Final flush: attempting to deliver {remaining} cached events");
        }
        while let Some(cached) = self.state.retry_queue.pop_front() {
            if let Err(e) = self.publish_event(&cached.event) {
                warn!(
                    "Final flush failed for event after {} attempts: {:?}",
                    cached.attempts, e
                );
            } else {
                debug!(
                    "Final flush: event delivered after {} attempts",
                    cached.attempts
                );
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(not(miri))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects,
    clippy::match_wild_err_arm
)]
mod tests {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    use common::fault::FaultId;
    use serial_test::serial;

    use super::*;
    use crate::{fault_manager_sink::WorkerMsg, test_utils::*, utils::to_static_long_string};

    // ---------- Helpers ----------

    /// Create a stub `DiagnosticEvent::Fault` for testing.
    fn stub_fault_event() -> DiagnosticEvent {
        let path = to_static_long_string("test/retry/path").unwrap();
        let desc = stub_descriptor(
            FaultId::Numeric(42),
            crate::utils::to_static_short_string("RetryTest").unwrap(),
            None,
            None,
        );
        DiagnosticEvent::Fault((path, stub_record(desc)))
    }

    fn make_state(config: RetryConfig) -> IpcWorkerState {
        IpcWorkerState::new(config)
    }

    // ---------- RetryConfig defaults ----------

    #[test]
    fn retry_config_defaults_are_sensible() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 10);
        assert_eq!(config.cache_capacity, 512);
        assert_eq!(config.retry_interval, Duration::from_millis(100));
        assert_eq!(config.max_retry_interval, Duration::from_secs(5));
    }

    // ---------- CachedFault backoff ----------

    #[test]
    fn backoff_delay_grows_exponentially() {
        let config = RetryConfig::default();
        let mut cached = CachedFault::new(stub_fault_event());

        // attempts=0 → 100ms * 2^0 = 100ms
        assert_eq!(cached.backoff_delay(&config), Duration::from_millis(100));

        cached.attempts = 1;
        assert_eq!(cached.backoff_delay(&config), Duration::from_millis(200));

        cached.attempts = 2;
        assert_eq!(cached.backoff_delay(&config), Duration::from_millis(400));

        cached.attempts = 3;
        assert_eq!(cached.backoff_delay(&config), Duration::from_millis(800));
    }

    #[test]
    fn backoff_delay_caps_at_max() {
        let config = RetryConfig {
            max_retry_interval: Duration::from_secs(5),
            ..Default::default()
        };
        let mut cached = CachedFault::new(stub_fault_event());

        // 100ms * 2^10 = 102400ms ≫ 5s cap
        cached.attempts = 10;
        assert_eq!(cached.backoff_delay(&config), Duration::from_secs(5));
    }

    // ---------- is_transient ----------

    #[test]
    fn transient_errors_classified_correctly() {
        assert!(IpcWorkerState::is_transient(&SinkError::TransportDown));
        assert!(IpcWorkerState::is_transient(&SinkError::Timeout));
        assert!(IpcWorkerState::is_transient(&SinkError::QueueFull));
        assert!(IpcWorkerState::is_transient(&SinkError::RateLimited));
    }

    #[test]
    fn permanent_errors_classified_correctly() {
        assert!(!IpcWorkerState::is_transient(&SinkError::PermissionDenied));
        assert!(!IpcWorkerState::is_transient(&SinkError::BadDescriptor(
            "test".into()
        )));
        assert!(!IpcWorkerState::is_transient(
            &SinkError::InvalidServiceName
        ));
    }

    // ---------- cache_for_retry ----------

    #[test]
    fn cache_for_retry_adds_to_queue() {
        let mut state = make_state(RetryConfig::default());
        assert_eq!(state.retry_queue_len(), 0);

        state.cache_for_retry(stub_fault_event());
        assert_eq!(state.retry_queue_len(), 1);

        state.cache_for_retry(stub_fault_event());
        assert_eq!(state.retry_queue_len(), 2);
    }

    #[test]
    fn cache_for_retry_evicts_oldest_when_full() {
        let mut state = make_state(RetryConfig {
            cache_capacity: 2,
            ..Default::default()
        });

        state.cache_for_retry(stub_fault_event());
        state.cache_for_retry(stub_fault_event());
        assert_eq!(state.retry_queue_len(), 2);

        // Third entry should evict the oldest
        state.cache_for_retry(stub_fault_event());
        assert_eq!(state.retry_queue_len(), 2);
    }

    // ---------- handle_send_failure ----------

    #[test]
    fn handle_send_failure_caches_on_transient_error() {
        let mut state = make_state(RetryConfig::default());

        state.handle_send_failure(stub_fault_event(), &SinkError::TransportDown);
        assert_eq!(state.retry_queue_len(), 1);
    }

    #[test]
    fn handle_send_failure_drops_on_permanent_error() {
        let mut state = make_state(RetryConfig::default());

        state.handle_send_failure(stub_fault_event(), &SinkError::PermissionDenied);
        assert_eq!(state.retry_queue_len(), 0);
    }

    #[test]
    fn handle_send_failure_caches_on_rate_limited() {
        let mut state = make_state(RetryConfig::default());

        state.handle_send_failure(stub_fault_event(), &SinkError::RateLimited);
        assert_eq!(
            state.retry_queue_len(),
            1,
            "RateLimited is transient, event should be cached for retry"
        );
    }

    // ---------- process_retries ----------

    #[test]
    fn process_retries_succeeds_on_first_retry() {
        let mut state = make_state(RetryConfig::default());
        state.cache_for_retry(stub_fault_event());

        let call_count = AtomicUsize::new(0);
        state.process_retries(&|_event| {
            call_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            state.retry_queue_len(),
            0,
            "Queue should be empty after successful retry"
        );
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn process_retries_requeues_on_transient_failure() {
        let mut state = make_state(RetryConfig {
            max_retries: 5,
            ..Default::default()
        });
        state.cache_for_retry(stub_fault_event());

        // First process: fails, requeued
        state.process_retries(&|_event| Err(SinkError::TransportDown));
        assert_eq!(
            state.retry_queue_len(),
            1,
            "Should requeue on transient failure"
        );

        // The requeued item has attempts=1 and a future next_retry
        let cached = &state.retry_queue[0];
        assert_eq!(cached.attempts, 1);
        assert!(cached.next_retry > Instant::now());
    }

    #[test]
    fn process_retries_drops_after_max_retries() {
        let config = RetryConfig {
            max_retries: 3,
            // Use zero retry interval so all retries are immediately eligible
            retry_interval: Duration::ZERO,
            ..Default::default()
        };
        let mut state = make_state(config);
        state.cache_for_retry(stub_fault_event());

        // Process 3 times — each time fails, last time drops
        for _ in 0..3 {
            state.process_retries(&|_event| Err(SinkError::TransportDown));
        }

        assert_eq!(state.retry_queue_len(), 0, "Should drop after max_retries");
    }

    #[test]
    fn process_retries_respects_backoff_timing() {
        let mut state = make_state(RetryConfig {
            max_retries: 5,
            retry_interval: Duration::from_millis(200),
            ..Default::default()
        });
        state.cache_for_retry(stub_fault_event());

        // First process: fail → requeued with next_retry ~200ms in future
        state.process_retries(&|_event| Err(SinkError::TransportDown));
        assert_eq!(state.retry_queue_len(), 1);

        // Immediate second process: backoff not elapsed, should NOT retry
        let call_count = AtomicUsize::new(0);
        state.process_retries(&|_event| {
            call_count.fetch_add(1, Ordering::SeqCst);
            Err(SinkError::TransportDown)
        });
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            0,
            "Should not retry before backoff elapses"
        );
        assert_eq!(state.retry_queue_len(), 1, "Event should remain in queue");
    }

    #[test]
    fn process_retries_with_flakey_publisher() {
        let fail_count = AtomicUsize::new(0);
        let config = RetryConfig {
            max_retries: 10,
            // Zero interval so retries are immediately eligible
            retry_interval: Duration::ZERO,
            max_retry_interval: Duration::ZERO,
            ..Default::default()
        };
        let mut state = make_state(config);
        state.cache_for_retry(stub_fault_event());

        // Fail twice, then succeed
        let attempts = AtomicUsize::new(0);
        let fail_times = 2;

        // Round 1: fail
        state.process_retries(&|_event| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < fail_times {
                fail_count.fetch_add(1, Ordering::SeqCst);
                Err(SinkError::TransportDown)
            } else {
                Ok(())
            }
        });
        assert_eq!(state.retry_queue_len(), 1); // Still queued (failed)

        // Round 2: fail
        state.process_retries(&|_event| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < fail_times {
                fail_count.fetch_add(1, Ordering::SeqCst);
                Err(SinkError::TransportDown)
            } else {
                Ok(())
            }
        });
        assert_eq!(state.retry_queue_len(), 1); // Still queued (failed again)

        // Round 3: succeed
        state.process_retries(&|_event| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < fail_times {
                fail_count.fetch_add(1, Ordering::SeqCst);
                Err(SinkError::TransportDown)
            } else {
                Ok(())
            }
        });
        assert_eq!(
            state.retry_queue_len(),
            0,
            "Should be delivered after retries"
        );
        assert_eq!(fail_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn process_retries_handles_multiple_cached_events() {
        let config = RetryConfig {
            max_retries: 5,
            retry_interval: Duration::ZERO,
            max_retry_interval: Duration::ZERO,
            ..Default::default()
        };
        let mut state = make_state(config);

        // Cache 3 events
        for _ in 0..3 {
            state.cache_for_retry(stub_fault_event());
        }
        assert_eq!(state.retry_queue_len(), 3);

        // All succeed
        let call_count = AtomicUsize::new(0);
        state.process_retries(&|_event| {
            call_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 3);
        assert_eq!(state.retry_queue_len(), 0);
    }

    // ---------- IpcWorker::run integration (uses real iceoryx2) ----------
    //
    // These tests create real iceoryx2 shared-memory resources.
    // They MUST run serially to avoid resource conflicts.
    // Each test uses a unique service name for defense-in-depth isolation.

    #[test]
    #[serial(ipc)]
    fn worker_start_and_exit_with_retry_config() {
        let svc = unique_ipc_service_name("worker_start_exit");
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let mut worker = IpcWorker::with_test_service(rx, RetryConfig::default(), &svc).unwrap();
        let handle = thread::spawn(move || worker.run());

        tx.send(WorkerMsg::Start(thread::current())).unwrap();
        tx.send(WorkerMsg::Exit).unwrap();

        let (join_tx, join_rx) = mpsc::channel();

        thread::spawn(move || {
            let join_result = handle.join();
            join_tx.send(join_result).ok();
        });

        let test_timeout = Duration::from_secs(5);
        match join_rx.recv_timeout(test_timeout) {
            Ok(Ok(())) => {}
            Ok(Err(panic_err)) => {
                std::panic::resume_unwind(panic_err);
            }
            Err(_) => {
                panic!("Test failed: Worker thread did not exit within 5 seconds");
            }
        }
    }

    #[test]
    #[serial(ipc)]
    fn worker_exits_on_channel_disconnect() {
        let svc = unique_ipc_service_name("worker_disconnect");
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let mut worker = IpcWorker::with_test_service(rx, RetryConfig::default(), &svc).unwrap();
        let handle = thread::spawn(move || worker.run());

        tx.send(WorkerMsg::Start(thread::current())).unwrap();
        // Drop sender → channel disconnects → worker should exit
        drop(tx);

        let (join_tx, join_rx) = mpsc::channel();
        thread::spawn(move || {
            let join_result = handle.join();
            join_tx.send(join_result).ok();
        });

        match join_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(panic_err)) => std::panic::resume_unwind(panic_err),
            Err(_) => panic!("Worker did not exit after channel disconnect"),
        }
    }
}
