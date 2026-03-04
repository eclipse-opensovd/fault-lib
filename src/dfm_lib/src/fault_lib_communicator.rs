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

//! IPC communication layer between reporter applications and the DFM.
//!
//! [`Iceoryx2Transport`] is the default [`DfmTransport`](crate::transport::DfmTransport)
//! implementation that uses iceoryx2 zero-copy shared memory for:
//! - **Diagnostic events** — fault reports, hash checks, enabling-condition
//!   registrations/status changes.
//! - **Hash-check responses** — sent back to reporters after catalog
//!   verification.
//! - **Enabling-condition notifications** — broadcast to all `FaultLib`
//!   instances when a condition status changes.
//!
//! The generic [`run_dfm_loop`] function drives the DFM event loop using
//! any `DfmTransport` implementation, enabling alternative transports
//! (in-memory channels, network-based, etc.) without modifying the core
//! DFM logic.

use common::enabling_condition::EnablingConditionNotification;
use common::ipc_service_name::{
    DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME,
    DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME,
    ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME,
};
use common::ipc_service_type::ServiceType;
use common::sink_error::SinkError;
use common::types::DiagnosticEvent;

use crate::enabling_condition_registry::EnablingConditionRegistry;
use crate::fault_record_processor::FaultRecordProcessor;
use crate::query_server::DfmQueryServer;
use crate::sovd_fault_storage::SovdFaultStateStorage;
use crate::transport::DfmTransport;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use iceoryx2::node::NodeBuilder;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::{Node, NodeName, ServiceName};
use tracing::info;

const DIAGNOSTIC_FAULT_MANAGER_LISTENER_NODE_NAME: &str = "fault_listener_node";

/// Default DFM cycle time used by [`Iceoryx2Transport`].
pub const DEFAULT_DFM_CYCLE_TIME: Duration = Duration::from_millis(10);

/// Errors that can occur during [`Iceoryx2Transport`] initialization.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportInitError {
    /// iceoryx2 node creation failed.
    #[error("node creation failed: {0}")]
    NodeCreation(String),
    /// iceoryx2 service or port creation failed.
    #[error("service creation failed: {0}")]
    ServiceCreation(String),
}

/// iceoryx2-based [`DfmTransport`] implementation (production default).
///
/// Uses iceoryx2 zero-copy shared memory publishers/subscribers for
/// communication between the DFM and reporter applications (`FaultLib`).
pub struct Iceoryx2Transport {
    catalog_hash_response_publisher: Publisher<ServiceType, bool, ()>,
    ec_notification_publisher: Publisher<ServiceType, EnablingConditionNotification, ()>,
    diagnostic_event_subscriber: Subscriber<ServiceType, DiagnosticEvent, ()>,
    node: Node<ServiceType>,
}

/// Service names bundle for test isolation.
#[cfg(test)]
#[cfg(not(miri))]
pub(crate) struct TestServiceNames {
    pub event: String,
    pub hash_response: String,
    pub ec_notification: String,
}

impl Default for Iceoryx2Transport {
    /// Creates a new transport using [`try_new`](Iceoryx2Transport::try_new).
    ///
    /// # Panics
    ///
    /// Panics if iceoryx2 IPC service creation fails.
    #[allow(clippy::expect_used)]
    fn default() -> Self {
        Self::try_new().expect("Iceoryx2Transport initialization failed")
    }
}

impl Iceoryx2Transport {
    /// Create a new iceoryx2 transport with default service names.
    ///
    /// # Panics
    ///
    /// Panics if iceoryx2 IPC service creation fails. These are system-level
    /// failures during initialization (no shared memory, no permissions).
    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn new() -> Self {
        Self::try_new().expect("Iceoryx2Transport initialization failed")
    }

    /// Fallible constructor for iceoryx2 transport with default service names.
    ///
    /// Prefer this over [`new`](Self::new) when the caller can handle
    /// initialization failures gracefully.
    ///
    /// # Errors
    ///
    /// Returns [`TransportInitError`] if any iceoryx2 node, service, or
    /// port cannot be created.
    pub fn try_new() -> Result<Self, TransportInitError> {
        let node_name = NodeName::new(DIAGNOSTIC_FAULT_MANAGER_LISTENER_NODE_NAME)
            .map_err(|e| TransportInitError::NodeCreation(format!("{e:?}")))?;
        let node = NodeBuilder::new()
            .name(&node_name)
            .create::<ServiceType>()
            .map_err(|e| TransportInitError::NodeCreation(format!("{e:?}")))?;

        let diagnostic_event_subscriber_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME)
                .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let diagnostic_event_subscriber_service = node
            .service_builder(&diagnostic_event_subscriber_service_name)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let diagnostic_event_subscriber = diagnostic_event_subscriber_service
            .subscriber_builder()
            .create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;

        let hash_response_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME)
                .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let hash_response_service = node
            .service_builder(&hash_response_service_name)
            .publish_subscribe::<bool>()
            .open_or_create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let catalog_hash_response_publisher = hash_response_service
            .publisher_builder()
            .create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;

        // Enabling condition notification publisher (DFM → FaultLib)
        let ec_notification_service_name =
            ServiceName::new(ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME)
                .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let ec_notification_service = node
            .service_builder(&ec_notification_service_name)
            .publish_subscribe::<EnablingConditionNotification>()
            .open_or_create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;
        let ec_notification_publisher = ec_notification_service
            .publisher_builder()
            .create()
            .map_err(|e| TransportInitError::ServiceCreation(format!("{e:?}")))?;

        Ok(Iceoryx2Transport {
            catalog_hash_response_publisher,
            ec_notification_publisher,
            diagnostic_event_subscriber,
            node,
        })
    }

    /// Create an iceoryx2 transport with custom service names for test isolation.
    ///
    /// Each test can supply unique service names to avoid iceoryx2 shared
    /// memory conflicts when tests run in parallel.
    #[cfg(test)]
    #[cfg(not(miri))]
    #[allow(clippy::expect_used, clippy::unwrap_used)]
    pub(crate) fn with_test_services(names: &TestServiceNames) -> Self {
        let node_name = NodeName::new("test_fault_listener_node").unwrap();
        let node = NodeBuilder::new()
            .name(&node_name)
            .create::<ServiceType>()
            .expect("Failed to create test listener node");

        let event_svc_name = ServiceName::new(&names.event).unwrap();
        let event_service = node
            .service_builder(&event_svc_name)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .expect("Failed to create test event service");
        let diagnostic_event_subscriber = event_service
            .subscriber_builder()
            .create()
            .expect("Failed to create test subscriber");

        let hash_svc_name = ServiceName::new(&names.hash_response).unwrap();
        let hash_service = node
            .service_builder(&hash_svc_name)
            .publish_subscribe::<bool>()
            .open_or_create()
            .expect("Failed to create test hash service");
        let catalog_hash_response_publisher = hash_service
            .publisher_builder()
            .create()
            .expect("Failed to create test hash publisher");

        let ec_svc_name = ServiceName::new(&names.ec_notification).unwrap();
        let ec_service = node
            .service_builder(&ec_svc_name)
            .publish_subscribe::<EnablingConditionNotification>()
            .open_or_create()
            .expect("Failed to create test EC service");
        let ec_notification_publisher = ec_service
            .publisher_builder()
            .create()
            .expect("Failed to create test EC publisher");

        Iceoryx2Transport {
            catalog_hash_response_publisher,
            ec_notification_publisher,
            diagnostic_event_subscriber,
            node,
        }
    }
}

impl DfmTransport for Iceoryx2Transport {
    fn receive_event(&self) -> Result<Option<DiagnosticEvent>, SinkError> {
        match self.diagnostic_event_subscriber.receive() {
            Ok(Some(sample)) => Ok(Some(sample.payload().clone())),
            Ok(None) => Ok(None),
            Err(_) => Err(SinkError::TransportDown),
        }
    }

    fn publish_hash_response(&self, response: bool) -> Result<(), SinkError> {
        let sample = self
            .catalog_hash_response_publisher
            .loan_uninit()
            .map_err(|_| SinkError::TransportDown)?;
        let sample = sample.write_payload(response);
        sample
            .send()
            .map_err(|_| SinkError::TransportDown)
            .map(|_| ())
    }

    fn publish_ec_notification(
        &self,
        notification: EnablingConditionNotification,
    ) -> Result<(), SinkError> {
        let sample = self
            .ec_notification_publisher
            .loan_uninit()
            .map_err(|_| SinkError::TransportDown)?;
        let sample = sample.write_payload(notification);
        sample
            .send()
            .map_err(|_| SinkError::TransportDown)
            .map(|_| ())
    }

    fn wait(&self, timeout: Duration) -> Result<bool, SinkError> {
        match self.node.wait(timeout) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

/// Optional extensions for the DFM event loop.
///
/// Groups optional services to avoid growing `run_dfm_loop`'s parameter list
/// each time a new capability is added.
pub struct DfmLoopExtensions<'a, S: SovdFaultStateStorage> {
    /// Query server for handling external SOVD query/clear requests.
    pub query_server: Option<&'a DfmQueryServer<S>>,
}

/// Run the DFM event loop using a generic [`DfmTransport`].
///
/// This is the core DFM run-loop extracted from the former
/// `FaultLibCommunicator::run_with_provider`. It is transport-agnostic:
/// any implementation of [`DfmTransport`] can be used.
///
/// The loop polls for events at `cycle_time` intervals (default:
/// [`DEFAULT_DFM_CYCLE_TIME`]) and processes them through the
/// `FaultRecordProcessor` and `EnablingConditionRegistry`.
#[allow(clippy::too_many_arguments)]
pub fn run_dfm_loop<T: DfmTransport, S: SovdFaultStateStorage>(
    transport: &T,
    shutdown: &AtomicBool,
    processor: &mut FaultRecordProcessor<S>,
    ec_registry: &mut EnablingConditionRegistry,
    cycle_provider: Option<
        &Arc<std::sync::Mutex<Box<dyn crate::operation_cycle::OperationCycleProvider>>>,
    >,
    cycle_tracker: &Arc<std::sync::RwLock<crate::operation_cycle::OperationCycleTracker>>,
    cycle_time: Duration,
    extensions: &DfmLoopExtensions<'_, S>,
) {
    info!("DFM transport listening...");
    while !shutdown.load(Ordering::Acquire) {
        // Wait for one cycle. Returns false if the transport node died.
        match transport.wait(cycle_time) {
            Ok(true) => {}
            Ok(false) => {
                info!("Transport node died, exiting DFM loop");
                break;
            }
            Err(e) => {
                tracing::error!("Transport wait error: {e:?}");
                break;
            }
        }

        // Poll operation-cycle provider (if attached) and apply events.
        if let Some(provider_arc) = cycle_provider {
            // Mutex lock scope is intentionally narrow to avoid holding
            // the lock while processing IPC messages.
            let events = {
                let mut provider = provider_arc
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                provider.poll()
            };
            if !events.is_empty() {
                let mut tracker = cycle_tracker
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let incremented = tracker.apply_events(&events);
                for name in &incremented {
                    tracing::trace!("Operation cycle '{name}' incremented via provider");
                }
            }
        }

        // Drain all available messages
        loop {
            let event = match transport.receive_event() {
                Ok(Some(e)) => e,
                Ok(None) => break, // no more messages
                Err(e) => {
                    tracing::error!("IPC receive error: {e:?}");
                    break;
                }
            };
            match &event {
                // NOTE: Enabling conditions are informational (monitor/callback pattern),
                // not enforcement gates. Faults are processed regardless of condition
                // status. If gating is needed, check ec_registry.is_condition_met()
                // before process_record() and skip with a trace! log.
                DiagnosticEvent::Fault((path, fault)) => {
                    info!("Received new fault ID: {:?}", fault.id);
                    processor.process_record(path, fault);
                }
                DiagnosticEvent::Hash((path, hash_sum)) => {
                    let result = processor.check_hash_sum(path, hash_sum);
                    info!("Received hash: {hash_sum:?}");
                    transport.publish_hash_response(result).unwrap_or_else(|e| {
                        tracing::error!("Failed to publish hash response: {e:?}");
                    });
                }
                DiagnosticEvent::EnablingConditionRegister(entity) => {
                    let entity_str = entity.to_string();
                    info!("Received enabling condition registration: {entity_str}");
                    ec_registry.register(&entity_str);
                }
                DiagnosticEvent::EnablingConditionStatusChange((id, status)) => {
                    let id_str = id.to_string();
                    info!("Received enabling condition status change: {id_str} -> {status:?}");
                    if let Some(new_status) = ec_registry.update_status(&id_str, *status) {
                        // Broadcast notification to all FaultLib subscribers
                        let notification = EnablingConditionNotification {
                            id: *id,
                            status: new_status,
                        };
                        if let Err(e) = transport.publish_ec_notification(notification) {
                            tracing::error!(
                                "Failed to publish EC notification for '{id_str}': {e:?}"
                            );
                        }
                    }
                }
            }
        }

        // Process query requests (SOVD query/clear from external apps)
        if let Some(qs) = extensions.query_server
            && let Err(e) = qs.poll()
        {
            tracing::error!("Query server poll error: {e:?}");
        }
    }
    info!("DFM transport loop shutdown complete");
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
    use super::*;
    use crate::dfm_test_utils::*;
    use crate::enabling_condition_registry::EnablingConditionRegistry;
    use crate::fault_record_processor::FaultRecordProcessor;
    use crate::sovd_fault_storage::SovdFaultStateStorage;
    use common::fault::{FaultId, LifecycleStage};
    use common::types::{LongString, to_static_short_string};
    use serial_test::serial;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU32;
    use std::thread;
    use std::time::Duration;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn unique_service_names(prefix: &str) -> TestServiceNames {
        let id = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pid = std::process::id();
        TestServiceNames {
            event: format!("test/{prefix}/{pid}/{id}/event"),
            hash_response: format!("test/{prefix}/{pid}/{id}/hash"),
            ec_notification: format!("test/{prefix}/{pid}/{id}/ec"),
        }
    }

    /// Send an event to the communicator via a publisher on the same service name.
    fn send_event(svc_name: &str, event: DiagnosticEvent) {
        let node = NodeBuilder::new().create::<ServiceType>().expect("node");
        let svc = ServiceName::new(svc_name).expect("svc name");
        let service = node
            .service_builder(&svc)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .expect("event service");
        let publisher = service.publisher_builder().create().expect("publisher");
        // Allow iceoryx2 discovery to complete before publishing
        thread::sleep(Duration::from_millis(50));
        let sample = publisher.loan_uninit().expect("loan");
        let sample = sample.write_payload(event);
        sample.send().expect("send");
    }

    // ---------- Startup / Shutdown ----------

    #[test]
    #[serial(ipc)]
    fn communicator_starts_and_shuts_down() {
        let names = unique_service_names("start_stop");
        let transport = Iceoryx2Transport::with_test_services(&names);
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let cycle_tracker = make_cycle_tracker();
        let processor = FaultRecordProcessor::new(storage, registry, Arc::clone(&cycle_tracker));
        let mut ec_registry = EnablingConditionRegistry::new();

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_dfm_loop(
                &transport,
                &shutdown_clone,
                &mut { processor },
                &mut ec_registry,
                None,
                &cycle_tracker,
                DEFAULT_DFM_CYCLE_TIME,
                &DfmLoopExtensions { query_server: None },
            );
        });

        // Let it run briefly
        thread::sleep(Duration::from_millis(50));
        shutdown.store(true, Ordering::Release);

        let (join_tx, join_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = join_tx.send(handle.join());
        });

        match join_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => std::panic::resume_unwind(e),
            Err(_) => panic!("Communicator did not shut down within 5 seconds"),
        }
    }

    // ---------- Fault Event Processing ----------

    #[test]
    #[serial(ipc)]
    fn communicator_processes_fault_event() {
        let names = unique_service_names("fault_event");
        let transport = Iceoryx2Transport::with_test_services(&names);
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let cycle_tracker = make_cycle_tracker();
        let processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&cycle_tracker));
        let mut ec_registry = EnablingConditionRegistry::new();

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_dfm_loop(
                &transport,
                &shutdown_clone,
                &mut { processor },
                &mut ec_registry,
                None,
                &cycle_tracker,
                DEFAULT_DFM_CYCLE_TIME,
                &DfmLoopExtensions { query_server: None },
            );
        });

        // Give the communicator time to start listening
        thread::sleep(Duration::from_millis(100));

        // Send a fault event via IPC
        let path = LongString::from_str_truncated("test_entity").unwrap();
        let record = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        let event = DiagnosticEvent::Fault((path, record));
        send_event(&names.event, event);

        // Wait for processing
        thread::sleep(Duration::from_millis(200));
        shutdown.store(true, Ordering::Release);

        let (join_tx, join_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = join_tx.send(handle.join());
        });
        join_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();

        // Verify the fault was stored
        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(
            state.is_some(),
            "Communicator should have processed the fault event"
        );
        let state = state.unwrap();
        assert!(state.test_failed);
        assert!(state.confirmed_dtc);
    }

    // ---------- Enabling Condition Registration ----------

    #[test]
    #[serial(ipc)]
    fn communicator_handles_ec_registration() {
        let names = unique_service_names("ec_reg");
        let transport = Iceoryx2Transport::with_test_services(&names);
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let cycle_tracker = make_cycle_tracker();
        let processor = FaultRecordProcessor::new(storage, registry, Arc::clone(&cycle_tracker));

        let ec_registry = Arc::new(std::sync::Mutex::new(EnablingConditionRegistry::new()));
        let ec_for_thread = Arc::clone(&ec_registry);

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            let mut ec = ec_for_thread.lock().unwrap();
            run_dfm_loop(
                &transport,
                &shutdown_clone,
                &mut { processor },
                &mut ec,
                None,
                &cycle_tracker,
                DEFAULT_DFM_CYCLE_TIME,
                &DfmLoopExtensions { query_server: None },
            );
        });

        thread::sleep(Duration::from_millis(50));

        // Send EC registration event
        let entity = to_static_short_string("engine.running").unwrap();
        let event = DiagnosticEvent::EnablingConditionRegister(entity);
        send_event(&names.event, event);

        thread::sleep(Duration::from_millis(100));
        shutdown.store(true, Ordering::Release);

        let (join_tx, join_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = join_tx.send(handle.join());
        });
        join_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
    }

    // ---------- Immediate Shutdown ----------

    #[test]
    #[serial(ipc)]
    fn communicator_exits_immediately_on_shutdown() {
        let names = unique_service_names("immediate_stop");
        let transport = Iceoryx2Transport::with_test_services(&names);
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let cycle_tracker = make_cycle_tracker();
        let processor = FaultRecordProcessor::new(storage, registry, Arc::clone(&cycle_tracker));
        let mut ec_registry = EnablingConditionRegistry::new();

        // Pre-set shutdown before starting
        let shutdown = Arc::new(AtomicBool::new(true));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_dfm_loop(
                &transport,
                &shutdown_clone,
                &mut { processor },
                &mut ec_registry,
                None,
                &cycle_tracker,
                DEFAULT_DFM_CYCLE_TIME,
                &DfmLoopExtensions { query_server: None },
            );
        });

        let (join_tx, join_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = join_tx.send(handle.join());
        });

        match join_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => std::panic::resume_unwind(e),
            Err(_) => panic!("Communicator should exit immediately when shutdown is pre-set"),
        }
    }

    // ---------- Multiple Messages ----------

    #[test]
    #[serial(ipc)]
    fn communicator_processes_multiple_events() {
        let names = unique_service_names("multi_event");
        let transport = Iceoryx2Transport::with_test_services(&names);
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let cycle_tracker = make_cycle_tracker();
        let processor =
            FaultRecordProcessor::new(Arc::clone(&storage), registry, Arc::clone(&cycle_tracker));
        let mut ec_registry = EnablingConditionRegistry::new();

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = thread::spawn(move || {
            run_dfm_loop(
                &transport,
                &shutdown_clone,
                &mut { processor },
                &mut ec_registry,
                None,
                &cycle_tracker,
                DEFAULT_DFM_CYCLE_TIME,
                &DfmLoopExtensions { query_server: None },
            );
        });

        thread::sleep(Duration::from_millis(100));

        // Send two fault events
        let path = LongString::from_str_truncated("test_entity").unwrap();
        let record_a = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        let record_b = make_record(
            FaultId::Text(to_static_short_string("fault_b").unwrap()),
            LifecycleStage::Passed,
        );
        send_event(&names.event, DiagnosticEvent::Fault((path, record_a)));
        send_event(&names.event, DiagnosticEvent::Fault((path, record_b)));

        thread::sleep(Duration::from_millis(200));
        shutdown.store(true, Ordering::Release);

        let (join_tx, join_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let _ = join_tx.send(handle.join());
        });
        join_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();

        let state_a = storage
            .get(
                "test_entity",
                &FaultId::Text(to_static_short_string("fault_a").unwrap()),
            )
            .unwrap();
        assert!(state_a.is_some(), "fault_a should be stored");
        assert!(state_a.unwrap().test_failed);

        let state_b = storage
            .get(
                "test_entity",
                &FaultId::Text(to_static_short_string("fault_b").unwrap()),
            )
            .unwrap();
        assert!(state_b.is_some(), "fault_b should be stored");
        assert!(!state_b.unwrap().test_failed);
    }
}
