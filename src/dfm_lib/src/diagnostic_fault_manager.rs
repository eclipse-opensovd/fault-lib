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

//! Top-level Diagnostic Fault Manager (DFM) orchestrator.
//!
//! [`DiagnosticFaultManager`] wires together the IPC transport,
//! fault-record processor, catalog/enabling-condition registries,
//! operation-cycle tracker, and SOVD fault manager into a single
//! run-loop that processes incoming diagnostic events from distributed
//! reporter applications.
//!
//! The DFM is generic over both the storage backend
//! ([`SovdFaultStateStorage`]) and the IPC transport ([`DfmTransport`]).
//! The default transport is [`Iceoryx2Transport`] (iceoryx2 shared memory).

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use std::{
    sync::{Mutex, RwLock},
    thread::{self, JoinHandle},
};

use tracing::{error, info};

use crate::{
    enabling_condition_registry::EnablingConditionRegistry,
    fault_catalog_registry::FaultCatalogRegistry,
    fault_lib_communicator::{
        DEFAULT_DFM_CYCLE_TIME, DfmLoopExtensions, Iceoryx2Transport, run_dfm_loop,
    },
    fault_record_processor::FaultRecordProcessor,
    operation_cycle::{OperationCycleProvider, OperationCycleTracker},
    query_server::DfmQueryServer,
    sovd_fault_manager::SovdFaultManager,
    sovd_fault_storage::SovdFaultStateStorage,
    transport::DfmTransport,
};

/// Central DFM orchestrator, generic over storage `S` and transport `T`.
///
/// By default (via [`new`](DiagnosticFaultManager::new) and
/// [`with_cycle_provider`](DiagnosticFaultManager::with_cycle_provider)),
/// the transport is [`Iceoryx2Transport`]. Use
/// [`with_transport`](DiagnosticFaultManager::with_transport) to inject
/// a custom [`DfmTransport`] implementation.
pub struct DiagnosticFaultManager<S: SovdFaultStateStorage, T: DfmTransport = Iceoryx2Transport> {
    shutdown: Arc<AtomicBool>,
    fault_lib_receiver_thread: Option<JoinHandle<()>>,
    storage: Arc<S>,
    registry: Arc<FaultCatalogRegistry>,
    cycle_tracker: Arc<RwLock<OperationCycleTracker>>,
    cycle_provider: Option<Arc<Mutex<Box<dyn OperationCycleProvider>>>>,
    _transport: core::marker::PhantomData<T>,
}

impl<S: SovdFaultStateStorage + 'static> DiagnosticFaultManager<S, Iceoryx2Transport> {
    /// Create a new `DiagnosticFaultManager` with default iceoryx2 transport.
    ///
    /// # Panics
    ///
    /// Panics if the receiver thread cannot be spawned. This is a system-level
    /// failure that indicates a fundamentally broken environment.
    #[allow(clippy::expect_used)]
    #[tracing::instrument(skip(storage, registry))]
    pub fn new(storage: S, registry: FaultCatalogRegistry) -> Self {
        Self::with_cycle_provider(storage, registry, None)
    }

    /// Create a DFM with an explicit [`OperationCycleProvider`] and default
    /// iceoryx2 transport.
    ///
    /// When a provider is set, each DFM iteration polls it for new events
    /// and feeds them into the shared [`OperationCycleTracker`]. This is the
    /// preferred way to integrate external lifecycle signals (ECU, HPC, etc.).
    ///
    /// # Panics
    ///
    /// Panics if the receiver thread cannot be spawned.
    #[allow(clippy::expect_used)]
    pub fn with_cycle_provider(
        storage: S,
        registry: FaultCatalogRegistry,
        provider: Option<Box<dyn OperationCycleProvider>>,
    ) -> Self {
        Self::with_transport(storage, registry, provider, false, Iceoryx2Transport::new)
    }

    /// Create a DFM with query server enabled for external IPC access.
    ///
    /// When enabled, the DFM loop polls for incoming [`DfmQueryRequest`](common::query_protocol::DfmQueryRequest)s
    /// and responds via iceoryx2 request-response on the `dfm/query` service.
    ///
    /// # Panics
    ///
    /// Panics if the receiver thread cannot be spawned.
    #[allow(clippy::expect_used)]
    pub fn with_query_server(storage: S, registry: FaultCatalogRegistry) -> Self {
        Self::with_transport(storage, registry, None, true, Iceoryx2Transport::new)
    }
}

impl<S: SovdFaultStateStorage + 'static, T: DfmTransport> DiagnosticFaultManager<S, T> {
    /// Create a DFM with a custom [`DfmTransport`] implementation.
    ///
    /// `transport_factory` is called on the worker thread to create the
    /// transport (some transports, e.g. iceoryx2, must be created on the
    /// thread that will use them).
    ///
    /// When `enable_query_server` is `true`, a [`DfmQueryServer`] is created
    /// on a separate iceoryx2 node inside the worker thread and polled each
    /// event-loop iteration.
    ///
    /// # Panics
    ///
    /// Panics if the receiver thread cannot be spawned.
    #[allow(clippy::expect_used)]
    pub fn with_transport(
        storage: S,
        registry: FaultCatalogRegistry,
        provider: Option<Box<dyn OperationCycleProvider>>,
        enable_query_server: bool,
        transport_factory: impl FnOnce() -> T + Send + 'static,
    ) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let storage = Arc::new(storage);
        let registry = Arc::new(registry);
        let cycle_tracker = Arc::new(RwLock::new(OperationCycleTracker::new()));
        let processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            Arc::clone(&cycle_tracker),
        );

        let cycle_provider: Option<Arc<Mutex<Box<dyn OperationCycleProvider>>>> =
            provider.map(|p| Arc::new(Mutex::new(p)));
        let worker_cycle_provider = cycle_provider.clone();
        let worker_cycle_tracker = Arc::clone(&cycle_tracker);
        let worker_storage = Arc::clone(&storage);
        let worker_registry = Arc::clone(&registry);

        let worker_shutdown = Arc::clone(&shutdown);
        let handle: JoinHandle<()> = thread::Builder::new()
            .name("fault_lib_receiver_thread".into())
            .spawn(move || {
                let mut ec_registry = EnablingConditionRegistry::new();
                let transport = transport_factory();

                let query_server = if enable_query_server {
                    match iceoryx2::node::NodeBuilder::new()
                        .create::<common::ipc_service_type::ServiceType>()
                    {
                        Ok(query_node) => {
                            let sovd = SovdFaultManager::new(worker_storage, worker_registry);
                            match DfmQueryServer::new(&query_node, sovd) {
                                Ok(server) => Some(server),
                                Err(e) => {
                                    error!(
                                        "Failed to create DfmQueryServer, query/clear disabled: {e}"
                                    );
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to create query server node, query/clear disabled: {e}");
                            None
                        }
                    }
                } else {
                    None
                };

                let extensions = DfmLoopExtensions {
                    query_server: query_server.as_ref(),
                };

                run_dfm_loop(
                    &transport,
                    &worker_shutdown,
                    &mut { processor },
                    &mut ec_registry,
                    worker_cycle_provider.as_ref(),
                    &worker_cycle_tracker,
                    DEFAULT_DFM_CYCLE_TIME,
                    &extensions,
                );
            })
            .expect("Failed to spawn the fault_lib_receiver_thread");

        Self {
            shutdown,
            fault_lib_receiver_thread: Some(handle),
            storage,
            registry,
            cycle_tracker,
            cycle_provider,
            _transport: core::marker::PhantomData,
        }
    }

    /// Provides shared access to the operation cycle tracker.
    ///
    /// External lifecycle events (power-on, ignition, etc.) should use this
    /// to increment the appropriate cycle counters, which in turn drive
    /// fault aging/reset evaluation.
    #[must_use]
    pub fn cycle_tracker(&self) -> &Arc<RwLock<OperationCycleTracker>> {
        &self.cycle_tracker
    }

    /// Provides shared access to the operation cycle provider, if one was set.
    #[must_use]
    pub fn cycle_provider(&self) -> Option<&Arc<Mutex<Box<dyn OperationCycleProvider>>>> {
        self.cycle_provider.as_ref()
    }

    #[must_use]
    pub fn create_sovd_fault_manager(&self) -> SovdFaultManager<S> {
        SovdFaultManager::new(Arc::clone(&self.storage), Arc::clone(&self.registry))
    }

    /// Returns a [`DirectDfmQuery`] - in-process implementation of [`DfmQueryApi`].
    ///
    /// This is the preferred way to obtain a query API handle when the SOVD
    /// consumer runs in the same process as the DFM.
    #[must_use]
    pub fn query_api(&self) -> crate::query_api::DirectDfmQuery<S> {
        crate::query_api::DirectDfmQuery::new(Arc::clone(&self.storage), Arc::clone(&self.registry))
    }
}

/// Timeout for joining the receiver thread during drop.
const DROP_JOIN_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(2);

impl<S: SovdFaultStateStorage, T: DfmTransport> Drop for DiagnosticFaultManager<S, T> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);

        if let Some(handle) = self.fault_lib_receiver_thread.take() {
            info!("Joining fault_lib_receiver_thread");
            let (join_tx, join_rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let result = handle.join();
                let _ = join_tx.send(result);
            });

            match join_rx.recv_timeout(DROP_JOIN_TIMEOUT) {
                Ok(Ok(())) => info!("fault_lib_receiver_thread done"),
                Ok(Err(err)) => error!("fault_lib_receiver_thread panicked: {err:?}"),
                Err(_) => error!(
                    "fault_lib_receiver_thread did not exit within {DROP_JOIN_TIMEOUT:?}, abandoning"
                ),
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[cfg(not(miri))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use serial_test::serial;

    use super::*;
    use crate::{dfm_test_utils::*, operation_cycle::ManualCycleProvider};

    /// Unwrap Arc<FaultCatalogRegistry> from test helpers into owned value.
    fn owned_registry() -> FaultCatalogRegistry {
        Arc::try_unwrap(make_text_registry())
            .ok()
            .expect("Arc has exactly one strong ref")
    }

    // ---------- Wiring and construction ----------

    #[test]
    #[serial(ipc)]
    fn dfm_creates_and_drops_cleanly() {
        let dfm = DiagnosticFaultManager::new(InMemoryStorage::new(), owned_registry());

        // No cycle provider when created with new()
        assert!(dfm.cycle_provider().is_none());

        // Drop should cleanly shut down the receiver thread
        drop(dfm);
    }

    // ---------- with_cycle_provider ----------

    #[test]
    #[serial(ipc)]
    fn dfm_with_cycle_provider_stores_provider() {
        let provider = ManualCycleProvider::new();
        let dfm = DiagnosticFaultManager::with_cycle_provider(
            InMemoryStorage::new(),
            owned_registry(),
            Some(Box::new(provider)),
        );

        assert!(dfm.cycle_provider().is_some());
        drop(dfm);
    }

    #[test]
    #[serial(ipc)]
    fn dfm_without_cycle_provider_returns_none() {
        let dfm = DiagnosticFaultManager::with_cycle_provider(
            InMemoryStorage::new(),
            owned_registry(),
            None,
        );

        assert!(dfm.cycle_provider().is_none());
        drop(dfm);
    }

    // ---------- create_sovd_fault_manager ----------

    #[test]
    #[serial(ipc)]
    fn dfm_returns_sovd_fault_manager() {
        let dfm = DiagnosticFaultManager::new(InMemoryStorage::new(), owned_registry());

        let sovd_manager = dfm.create_sovd_fault_manager();
        let faults = sovd_manager.get_all_faults("test_entity").unwrap();
        assert_eq!(
            faults.len(),
            2,
            "Should return all descriptors from catalog"
        );

        drop(dfm);
    }

    // ---------- cycle_tracker ----------

    #[test]
    #[serial(ipc)]
    fn dfm_cycle_tracker_is_accessible() {
        let dfm = DiagnosticFaultManager::new(InMemoryStorage::new(), owned_registry());

        let tracker = dfm.cycle_tracker();
        // Should be able to read from the tracker
        let read = tracker.read().unwrap();
        assert_eq!(read.get("power"), 0, "Empty tracker has no cycles");
        drop(read);

        // Should be able to write to the tracker
        let mut write = tracker.write().unwrap();
        write.increment("power");
        assert_eq!(write.get("power"), 1);
        drop(write);

        drop(dfm);
    }

    // ---------- Shutdown behavior ----------

    #[test]
    #[serial(ipc)]
    fn dfm_shutdown_is_idempotent() {
        // Create and immediately drop multiple times — should not panic
        let dfm1 = DiagnosticFaultManager::new(InMemoryStorage::new(), owned_registry());
        drop(dfm1);

        let dfm2 = DiagnosticFaultManager::new(InMemoryStorage::new(), owned_registry());
        drop(dfm2);
    }
}
