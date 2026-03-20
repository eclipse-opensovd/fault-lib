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

use alloc::sync::Weak;
use core::time::Duration;
use std::{
    sync::{mpsc, mpsc::TrySendError},
    thread::{self, JoinHandle},
    time::Instant,
};

use common::{
    fault::FaultRecord,
    ipc_service_name::DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME,
    ipc_service_type::ServiceType,
    sink_error::SinkError,
    types::{DiagnosticEvent, LONG_STRING_CAPACITY, Sha256Vec},
};
use iceoryx2::{
    port::subscriber::Subscriber,
    prelude::{NodeBuilder, ServiceName},
};
use tracing::{debug, error, warn};

use crate::{
    FaultApi, enabling_condition::EnablingConditionManager, ipc_worker::IpcWorker,
    sink::FaultSinkApi, utils::to_static_long_string,
};

/// Initialization errors for the IPC sink and worker.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SinkInitError {
    /// Worker thread could not be spawned.
    #[error("thread spawn failed: {0}")]
    ThreadSpawn(String),

    /// Worker thread failed during IPC resource setup.
    #[error("worker initialization failed: {0}")]
    WorkerInit(String),

    /// iceoryx2 node, service, or port creation failed.
    #[error("IPC service creation failed: {0}")]
    IpcService(String),

    /// Internal channel communication failed.
    #[error("channel error: {0}")]
    Channel(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FaultManagerError {
    SendError(String),
    Timeout,
}

impl From<FaultManagerError> for SinkError {
    fn from(err: FaultManagerError) -> Self {
        match err {
            FaultManagerError::SendError(msg) => SinkError::Other(alloc::borrow::Cow::Owned(msg)),
            FaultManagerError::Timeout => SinkError::Other(alloc::borrow::Cow::Borrowed("timeout")),
        }
    }
}

/// Request channel type used by the `sink_thread` to receive events
pub type WorkerReceiver = mpsc::Receiver<WorkerMsg>;

#[derive(Debug)]
pub(crate) enum WorkerMsg {
    /// transports start message with the parent thread which will be unparked when the `sink_thread` thread is up and running
    Start(std::thread::Thread),

    /// Sent by the `FaultManagerSink` when the fault monitor reports an event.
    Event { event: Box<DiagnosticEvent> },

    /// Terminate the `FaultManagerSink` working thread
    Exit,
}

const TIMEOUT: Duration = Duration::from_millis(500);

/// Maximum number of pending messages in the channel.
/// Provides backpressure when the IPC worker is slow to process.
const CHANNEL_CAPACITY: usize = 1024;

pub struct FaultManagerSink {
    sink_sender: mpsc::SyncSender<WorkerMsg>,
    sink_thread: Option<JoinHandle<()>>,
    hash_check_response_subscriber: Option<Subscriber<ServiceType, bool, ()>>,
}

impl FaultManagerSink {
    /// Create a new `FaultManagerSink` with IPC worker thread and hash check subscriber.
    ///
    /// # Errors
    ///
    /// Returns `SinkInitError` if thread spawn or iceoryx2 IPC service creation fails.
    #[allow(dead_code)]
    pub(crate) fn new() -> Result<Self, SinkInitError> {
        Self::with_ec_manager(Weak::<EnablingConditionManager>::new())
    }

    /// Create a new `FaultManagerSink` with a reference to the enabling condition manager.
    ///
    /// The EC manager is passed to the IPC worker so it can receive
    /// enabling condition notifications from the DFM.
    ///
    /// # Errors
    ///
    /// Returns `SinkInitError` if thread spawn or iceoryx2 IPC service creation fails.
    pub(crate) fn with_ec_manager(
        ec_manager: Weak<EnablingConditionManager>,
    ) -> Result<Self, SinkInitError> {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);

        // Channel for the worker to report its initialization result back.
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), SinkInitError>>(1);

        let handle = thread::Builder::new()
            .name("fault_client_worker".into())
            .spawn(move || match IpcWorker::new(rx, ec_manager) {
                Ok(mut ipc_worker) => {
                    let _ = init_tx.send(Ok(()));
                    ipc_worker.run();
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                }
            })
            .map_err(|e| SinkInitError::ThreadSpawn(e.to_string()))?;

        // Wait for worker to finish IPC resource setup.
        init_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| {
                SinkInitError::WorkerInit("timeout waiting for worker initialization".into())
            })?
            .map_err(|e| SinkInitError::WorkerInit(e.to_string()))?;

        tx.send(WorkerMsg::Start(thread::current()))
            .map_err(|e| SinkInitError::Channel(format!("start message: {e}")))?;
        // Use park_timeout to avoid hanging forever if the worker thread
        // panics before calling unpark. 5 seconds is generous for thread init.
        thread::park_timeout(Duration::from_secs(5));

        let node = NodeBuilder::new()
            .create::<ServiceType>()
            .map_err(|e| SinkInitError::IpcService(format!("hash check node: {e}")))?;
        let hash_check_response_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME)
                .map_err(|e| SinkInitError::IpcService(format!("hash check service name: {e}")))?;
        let hash_check_response_service = node
            .service_builder(&hash_check_response_service_name)
            .publish_subscribe::<bool>()
            .open_or_create()
            .map_err(|e| SinkInitError::IpcService(format!("hash check service: {e}")))?;
        let hash_check_response_subscriber = hash_check_response_service
            .subscriber_builder()
            .create()
            .map_err(|e| SinkInitError::IpcService(format!("hash check subscriber: {e}")))?;

        Ok(Self {
            sink_sender: tx,
            sink_thread: Some(handle),
            hash_check_response_subscriber: Some(hash_check_response_subscriber),
        })
    }

    /// Validate catalog hash against DFM without reading from global state.
    ///
    /// Used during initialization before the global `OnceLock` is committed,
    /// so that a hash mismatch doesn't leave the system in a partial state.
    pub(crate) fn check_catalog_hash(
        &self,
        catalog: &crate::catalog::FaultCatalog,
    ) -> Result<bool, SinkError> {
        let catalog_id = catalog.try_id().map_err(|e| {
            SinkError::Other(alloc::borrow::Cow::Owned(format!("catalog id error: {e}")))
        })?;
        let hash_vec = common::types::Sha256Vec::try_from(catalog.config_hash()).map_err(|_| {
            SinkError::Other(alloc::borrow::Cow::Borrowed(
                "catalog hash too long for IPC",
            ))
        })?;
        let event = common::types::DiagnosticEvent::Hash((catalog_id, hash_vec));
        match self.sink_sender.try_send(WorkerMsg::Event {
            event: Box::new(event),
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(SinkError::QueueFull),
            Err(TrySendError::Disconnected(_)) => {
                return Err(FaultManagerError::SendError(
                    "Cannot send hash check: channel disconnected".into(),
                )
                .into());
            }
        }
        self.listen_hash_check_response()
    }

    /// Listen for hash check response from DFM.
    ///
    /// Uses polling with 50ms interval. A future improvement would be
    /// to use iceoryx2 `WaitSet` for true event-driven notification.
    fn listen_hash_check_response(&self) -> Result<bool, SinkError> {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(50);

        let subscriber = self
            .hash_check_response_subscriber
            .as_ref()
            .ok_or(SinkError::Other(alloc::borrow::Cow::Borrowed(
                "hash check subscriber not initialized",
            )))?;

        while start.elapsed() < TIMEOUT {
            if let Some(msg) = subscriber.receive().map_err(|_| SinkError::TransportDown)? {
                return Ok(*msg.payload());
            }
            std::thread::sleep(poll_interval);
        }

        Err(SinkError::Timeout)
    }
}

/// Maximum path length for fault entity paths.
/// Compile-time guarantee: stays in sync with `LongString` capacity.
const MAX_PATH_LENGTH: usize = LONG_STRING_CAPACITY;
const _: () = assert!(MAX_PATH_LENGTH == LONG_STRING_CAPACITY);

/// Validate entity path format.
///
/// Valid paths must:
/// - Not be empty
/// - Start with alphanumeric character
/// - Not end with '/'
/// - Not contain path traversal sequences ("..")
/// - Contain only: alphanumeric, '/', '.', '-', '_'
fn is_valid_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if !path.starts_with(|c: char| c.is_alphanumeric()) {
        return false;
    }
    if path.ends_with('/') {
        return false;
    }
    if path.contains("..") {
        return false;
    }
    path.chars()
        .all(|c| c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
}

/// API to be used by the modules of the fault-lib which need to communicate with
/// Diagnostic Fault Manager. This trait shall never become public
impl FaultSinkApi for FaultManagerSink {
    fn send_event(&self, event: DiagnosticEvent) -> Result<(), SinkError> {
        match self.sink_sender.try_send(WorkerMsg::Event {
            event: Box::new(event),
        }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                warn!("Event queue full, dropping event");
                Err(SinkError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => Err(FaultManagerError::SendError(
                "Cannot send event: channel disconnected".into(),
            )
            .into()),
        }
    }

    /// Validate and enqueue a fault record for IPC delivery.
    ///
    /// The entity `path` is converted to a `LongString<128>` (`StaticString<128>`).
    /// Paths exceeding 128 bytes are rejected with [`SinkError::BadDescriptor`]
    /// rather than silently truncated.
    fn publish(&self, path: &str, record: FaultRecord) -> Result<(), SinkError> {
        // Validate path before IPC
        if path.len() > MAX_PATH_LENGTH {
            return Err(SinkError::BadDescriptor(alloc::borrow::Cow::Owned(
                format!(
                    "path too long: {} bytes (max {})",
                    path.len(),
                    MAX_PATH_LENGTH
                ),
            )));
        }
        if !is_valid_path(path) {
            return Err(SinkError::BadDescriptor(alloc::borrow::Cow::Borrowed(
                "invalid path format",
            )));
        }

        let long_path = to_static_long_string(path).map_err(|_| {
            SinkError::BadDescriptor(alloc::borrow::Cow::Owned(format!(
                "path exceeds LongString capacity: {} bytes",
                path.len()
            )))
        })?;
        let event = DiagnosticEvent::Fault((long_path, record));
        match self.sink_sender.try_send(WorkerMsg::Event {
            event: Box::new(event),
        }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                warn!("Fault queue full, dropping record");
                Err(SinkError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => Err(FaultManagerError::SendError(
                "Cannot send event: channel disconnected".into(),
            )
            .into()),
        }
    }

    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        let catalog = FaultApi::get_fault_catalog();
        let catalog_id = catalog.try_id().map_err(|e| {
            SinkError::Other(alloc::borrow::Cow::Owned(format!("catalog id error: {e}")))
        })?;
        let hash_vec = Sha256Vec::try_from(catalog.config_hash()).map_err(|_| {
            SinkError::Other(alloc::borrow::Cow::Borrowed(
                "catalog hash too long for IPC",
            ))
        })?;
        let event = DiagnosticEvent::Hash((catalog_id, hash_vec));
        match self.sink_sender.try_send(WorkerMsg::Event {
            event: Box::new(event),
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(SinkError::QueueFull),
            Err(TrySendError::Disconnected(_)) => {
                return Err(FaultManagerError::SendError(
                    "Cannot send hash check: channel disconnected".into(),
                )
                .into());
            }
        }
        // this will wait for the response
        self.listen_hash_check_response()
    }
}

/// Timeout for joining the worker thread during drop.
const DROP_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

impl Drop for FaultManagerSink {
    fn drop(&mut self) {
        debug!("Drop FaultManagerSink");
        if let Some(hndl) = self.sink_thread.take() {
            // try_send avoids blocking on a full channel during shutdown
            if let Err(e) = self.sink_sender.try_send(WorkerMsg::Exit) {
                debug!("Exit signal send failed (worker may have already exited): {e:?}");
            }

            let current_id = std::thread::current().id();
            let worker_id = hndl.thread().id();

            if current_id == worker_id {
                error!("Skipping join: drop called from the sink_thread thread");
                return;
            }

            debug!("Joining sink_thread thread");
            let (join_tx, join_rx) = std::sync::mpsc::channel();
            let watchdog = thread::spawn(move || {
                let result = hndl.join();
                let _ = join_tx.send(result);
            });

            match join_rx.recv_timeout(DROP_JOIN_TIMEOUT) {
                Ok(Ok(())) => debug!("Worker thread joined successfully"),
                Ok(Err(err)) => error!("Worker thread panicked: {err:?}"),
                Err(_) => {
                    error!("Worker thread did not exit within {DROP_JOIN_TIMEOUT:?}, abandoning");
                    drop(watchdog);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use common::{FaultId, types::*};

    use super::*;
    use crate::test_utils::*;

    fn new_for_publish_test() -> (FaultManagerSink, mpsc::Receiver<WorkerMsg>) {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        };
        (client, rx)
    }

    fn new_for_drop_test() -> (FaultManagerSink, mpsc::Receiver<WorkerMsg>) {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let handle = thread::spawn(|| {
            thread::sleep(Duration::from_millis(1));
        });
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: Some(handle),
            hash_check_response_subscriber: None,
        };
        (client, rx)
    }

    #[test]
    fn test_publish_sends_event_message() {
        let (client, rx) = new_for_publish_test();
        let fault_id = FaultId::Numeric(42);
        let fault_name = ShortString::from_bytes("Test Fault".as_bytes()).unwrap();
        let desc = stub_descriptor(fault_id, fault_name, None, None);
        let path = "test/path";

        let result =
            <FaultManagerSink as FaultSinkApi>::publish(&client, path, stub_record(desc.clone()));
        assert!(result.is_ok());

        match rx.recv_timeout(Duration::from_millis(50)).unwrap() {
            WorkerMsg::Event { event } => match &*event {
                DiagnosticEvent::Fault((path, record)) => {
                    assert_eq!(path.to_string(), "test/path");
                    assert_eq!(record.id, FaultId::Numeric(42));
                }
                other => {
                    panic!("Expected Fault event, got {other:?}");
                }
            },
            other => panic!("Received wrong message type: {other:?}"),
        }
    }

    #[test]
    fn test_drop_sends_exit_message() {
        let (client, rx) = new_for_drop_test();
        drop(client);

        match rx.recv_timeout(Duration::from_millis(50)).unwrap() {
            WorkerMsg::Exit => {}
            other => panic!("Received wrong message type, expected Exit: {other:?}"),
        }
    }

    // ---------- is_valid_path tests ----------

    #[test]
    fn path_traversal_rejected() {
        assert!(!is_valid_path("entity/../etc"));
        assert!(!is_valid_path("entity/.."));
        assert!(!is_valid_path("../entity"));
        assert!(!is_valid_path("a/b/../c"));
    }

    #[test]
    fn single_dots_in_path_allowed() {
        assert!(is_valid_path("entity/v1.2.3/sub"));
        assert!(is_valid_path("entity.name"));
        assert!(is_valid_path("a.b.c"));
    }

    #[test]
    fn empty_path_rejected() {
        assert!(!is_valid_path(""));
    }

    #[test]
    fn path_starting_with_slash_rejected() {
        assert!(!is_valid_path("/entity"));
    }

    #[test]
    fn path_ending_with_slash_rejected() {
        assert!(!is_valid_path("entity/"));
    }

    #[test]
    fn valid_paths_accepted() {
        assert!(is_valid_path("entity"));
        assert!(is_valid_path("test/path"));
        assert!(is_valid_path("a/b/c"));
        assert!(is_valid_path("entity-name_v1"));
    }

    #[test]
    fn path_with_special_chars_rejected() {
        assert!(!is_valid_path("entity path"));
        assert!(!is_valid_path("entity@name"));
        assert!(!is_valid_path("entity#1"));
    }

    // ---------- full queue behavior ----------

    #[test]
    fn publish_rejects_when_queue_is_full() {
        // Create a very small channel to fill up
        let (tx, _rx) = mpsc::sync_channel(1);
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        };

        let fault_name = ShortString::from_bytes("Test".as_bytes()).unwrap();
        let desc = stub_descriptor(FaultId::Numeric(1), fault_name, None, None);
        let path = "test/path";

        // First publish fills the channel
        let result =
            <FaultManagerSink as FaultSinkApi>::publish(&client, path, stub_record(desc.clone()));
        assert!(result.is_ok());

        // Second publish should fail with QueueFull
        let result = <FaultManagerSink as FaultSinkApi>::publish(&client, path, stub_record(desc));
        assert!(matches!(result, Err(SinkError::QueueFull)));
    }

    // ---------- invalid path handling ----------

    #[test]
    fn publish_rejects_path_too_long() {
        let (client, _rx) = new_for_publish_test();
        let fault_name = ShortString::from_bytes("Test".as_bytes()).unwrap();
        let desc = stub_descriptor(FaultId::Numeric(1), fault_name, None, None);

        // Path longer than MAX_PATH_LENGTH
        let long_path = "a".repeat(MAX_PATH_LENGTH + 1);
        let result =
            <FaultManagerSink as FaultSinkApi>::publish(&client, &long_path, stub_record(desc));
        assert!(matches!(result, Err(SinkError::BadDescriptor(_))));
    }

    #[test]
    fn publish_rejects_invalid_path_format() {
        let (client, _rx) = new_for_publish_test();
        let fault_name = ShortString::from_bytes("Test".as_bytes()).unwrap();
        let desc = stub_descriptor(FaultId::Numeric(1), fault_name, None, None);

        let result = <FaultManagerSink as FaultSinkApi>::publish(
            &client,
            "../etc/passwd",
            stub_record(desc.clone()),
        );
        assert!(matches!(result, Err(SinkError::BadDescriptor(_))));

        let result = <FaultManagerSink as FaultSinkApi>::publish(&client, "", stub_record(desc));
        assert!(matches!(result, Err(SinkError::BadDescriptor(_))));
    }

    // ---------- send_event ----------

    #[test]
    fn send_event_succeeds_with_open_channel() {
        let (client, _rx) = new_for_publish_test();
        let event = DiagnosticEvent::Hash((
            crate::utils::to_static_long_string("test").unwrap(),
            common::types::Sha256Vec::default(),
        ));
        let result = <FaultManagerSink as FaultSinkApi>::send_event(&client, event);
        assert!(result.is_ok());
    }

    #[test]
    fn send_event_fails_on_disconnected_channel() {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        };
        // Drop the receiver to disconnect the channel
        drop(rx);

        let event = DiagnosticEvent::Hash((
            crate::utils::to_static_long_string("test").unwrap(),
            common::types::Sha256Vec::default(),
        ));
        let result = <FaultManagerSink as FaultSinkApi>::send_event(&client, event);
        assert!(result.is_err());
    }

    // ---------- drop behavior ----------

    #[test]
    fn drop_without_thread_is_safe() {
        let (tx, _rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        };
        // Should not panic
        drop(client);
    }

    #[test]
    fn drop_with_disconnected_channel_is_safe() {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let handle = thread::spawn(|| {
            thread::sleep(Duration::from_millis(1));
        });
        drop(rx); // Disconnect receiver
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: Some(handle),
            hash_check_response_subscriber: None,
        };
        // Should not panic even though channel is disconnected
        drop(client);
    }

    // ---------- concurrent publish ----------

    #[test]
    fn concurrent_publish_does_not_panic() {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let client = Arc::new(FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        });

        // Spawn receiver that drains events
        let drain =
            thread::spawn(move || while rx.recv_timeout(Duration::from_millis(500)).is_ok() {});

        // Spawn multiple threads publishing concurrently
        let mut handles = vec![];
        for i in 0..4 {
            let client_clone = Arc::clone(&client);
            handles.push(thread::spawn(move || {
                for j in 0..10 {
                    let fault_name = ShortString::from_bytes("Test".as_bytes()).unwrap();
                    let desc =
                        stub_descriptor(FaultId::Numeric(i * 100 + j), fault_name, None, None);
                    let _ = <FaultManagerSink as FaultSinkApi>::publish(
                        &*client_clone,
                        "test/path",
                        stub_record(desc),
                    );
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
        drop(client); // Drop client to disconnect channel
        drain.join().unwrap();
    }

    // ---------- SinkInitError ----------

    #[test]
    fn sink_init_error_display() {
        let err = SinkInitError::ThreadSpawn("os error".into());
        assert!(err.to_string().contains("thread spawn failed"));

        let err = SinkInitError::WorkerInit("timeout".into());
        assert!(err.to_string().contains("worker initialization failed"));

        let err = SinkInitError::IpcService("no shm".into());
        assert!(err.to_string().contains("IPC service creation failed"));

        let err = SinkInitError::Channel("disconnected".into());
        assert!(err.to_string().contains("channel error"));
    }
}
