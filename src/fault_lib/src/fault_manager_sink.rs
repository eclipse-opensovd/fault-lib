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

use crate::FaultApi;
use crate::ipc_worker::IpcWorker;
use crate::sink::*;
use crate::utils::to_static_long_string;
use common::fault::FaultRecord;
use common::ipc_service_name::DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME;
use common::ipc_service_type::ServiceType;
use common::sink_error::SinkError;
use common::types::{DiagnosticEvent, Sha256Vec};
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::{NodeBuilder, ServiceName};
use log::*;
use std::time::{Duration, Instant};
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

#[allow(unused_imports)]
use mockall::automock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultManagerError {
    SendError(String),
    Timeout,
}

impl From<FaultManagerError> for SinkError {
    fn from(err: FaultManagerError) -> Self {
        match err {
            FaultManagerError::SendError(msg) => SinkError::Other(Box::leak(msg.into_boxed_str())),
            FaultManagerError::Timeout => SinkError::Other("timeout"),
        }
    }
}

/// Request channel type used by the sink_thread to receive events
pub type WorkerReceiver = mpsc::Receiver<WorkerMsg>;

#[derive(Debug)]
pub(crate) enum WorkerMsg {
    /// transports start message with the parent thread which will be unparked when the sink_thread thread is up and running
    Start(std::thread::Thread),

    /// Sent by the FaultManagerSink when the fault monitor reports an event.
    Event { event: Box<DiagnosticEvent> },

    /// Terminate the FaultManagerSink working thread
    Exit,
}

const TIMEOUT: Duration = Duration::from_millis(500);

pub struct FaultManagerSink {
    sink_sender: mpsc::Sender<WorkerMsg>,
    sink_thread: Option<JoinHandle<()>>,
    hash_check_response_subscriber: Option<Subscriber<ServiceType, bool, ()>>,
}

impl FaultManagerSink {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("fault_client_worker".into())
            .spawn(move || {
                let ipc_worker = IpcWorker::new(rx);
                ipc_worker.run();
            })
            .expect("Cannot spawn Fault Mgr Client sink thread");

        tx.send(WorkerMsg::Start(thread::current())).expect("Couldn't start the worker thread");
        thread::park();

        let node = NodeBuilder::new()
            .create::<ServiceType>()
            .expect("Failed to create the fault service node");
        let hash_check_response_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME).expect("Failed to create the fault service name");
        let hash_check_response_service = node
            .service_builder(&hash_check_response_service_name)
            .publish_subscribe::<bool>()
            .open_or_create()
            .expect("Failed to create the hash transmitter service");
        let hash_check_response_subscriber = hash_check_response_service
            .subscriber_builder()
            .create()
            .expect("Failed to create the hash transmitter client");
        Self {
            sink_sender: tx,
            sink_thread: Some(handle),
            hash_check_response_subscriber: Some(hash_check_response_subscriber),
        }
    }

    fn listen_hash_check_response(&self) -> Result<bool, SinkError> {
        let start = Instant::now();
        loop {
            if let Some(msg) = self
                .hash_check_response_subscriber
                .as_ref()
                .unwrap()
                .receive()
                .map_err(|_| SinkError::TransportDown)?
            {
                return Ok(*msg.payload());
            }
            if start.elapsed() >= TIMEOUT {
                return Err(SinkError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// API to be used by the modules of the fault-lib which need to communicate with
/// Diagnostic Fault Manager. This trait shall never become public
impl FaultSinkApi for FaultManagerSink {
    fn publish(&self, path: &str, record: FaultRecord) -> Result<(), SinkError> {
        let event = DiagnosticEvent::Fault((to_static_long_string(path).expect("Failed to serialize path"), record));
        self.sink_sender
            .send(WorkerMsg::Event { event: Box::new(event) })
            .map_err(|e| FaultManagerError::SendError(format!("Cannot send event: {e}")))?;
        Ok(())
    }

    fn check_fault_catalog(&self) -> Result<bool, SinkError> {
        let catalog = FaultApi::get_fault_catalog();
        let event = DiagnosticEvent::Hash((catalog.id(), Sha256Vec::try_from(catalog.config_hash()).unwrap()));
        // this send immediately returns
        let result = self
            .sink_sender
            .send(WorkerMsg::Event { event: Box::new(event) })
            .map_err(|e| SinkError::from(FaultManagerError::SendError(format!("Cannot send event: {e}"))));
        if result.is_err() {
            return Err(result.err().unwrap());
        }
        // this will wait for the response
        self.listen_hash_check_response()
    }
}

impl Drop for FaultManagerSink {
    fn drop(&mut self) {
        debug!("Drop FaultManagerSink");
        if let Some(hndl) = self.sink_thread.take() {
            let _ = self.sink_sender.send(WorkerMsg::Exit);

            let current_id = std::thread::current().id();
            let worker_id = hndl.thread().id();

            if current_id == worker_id {
                error!("Skipping join: drop called from the sink_thread thread");
                return;
            }

            debug!("Joining sink_thread thread");
            if let Err(err) = hndl.join() {
                error!("Worker thread panicked: {:?}", err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;
    use common::FaultId;
    use common::types::*;
    use std::time::Duration;

    fn new_for_publish_test() -> (FaultManagerSink, mpsc::Receiver<WorkerMsg>) {
        let (tx, rx) = mpsc::channel();
        let client = FaultManagerSink {
            sink_sender: tx,
            sink_thread: None,
            hash_check_response_subscriber: None,
        };
        (client, rx)
    }

    fn new_for_drop_test() -> (FaultManagerSink, mpsc::Receiver<WorkerMsg>) {
        let (tx, rx) = mpsc::channel();
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

        let result = <FaultManagerSink as FaultSinkApi>::publish(&client, path, stub_record(desc.clone()));
        assert!(result.is_ok());

        match rx.recv_timeout(Duration::from_millis(50)).unwrap() {
            WorkerMsg::Event { event } => match &*event {
                DiagnosticEvent::Fault((path, record)) => {
                    assert_eq!(path.to_string(), "test/path");
                    assert_eq!(record.id, FaultId::Numeric(42));
                }
                DiagnosticEvent::Hash(_) => {
                    panic!("Expected Fault event, got Hash");
                }
            },
            other => panic!("Received wrong message type: {:?}", other),
        }
    }

    #[test]
    fn test_drop_sends_exit_message() {
        let (client, rx) = new_for_drop_test();
        drop(client);

        match rx.recv_timeout(Duration::from_millis(50)).unwrap() {
            WorkerMsg::Exit => {}
            other => panic!("Received wrong message type, expected Exit: {:?}", other),
        }
    }
}
