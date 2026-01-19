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
use crate::fault_manager_sink::{WorkerMsg, WorkerReceiver};
use common::ipc_service_name::DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME;
use common::ipc_service_type::ServiceType;
use common::sink_error::SinkError;
use common::types::DiagnosticEvent;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::{NodeBuilder, ServiceName};
use log::*;

#[allow(unused_imports)]
use mockall::automock;

pub struct IpcWorker {
    #[allow(dead_code)]
    sink_receiver: WorkerReceiver,
    diagnostic_publisher: Option<Publisher<ServiceType, DiagnosticEvent, ()>>,
}

impl IpcWorker {
    pub fn new(sink_receiver: WorkerReceiver) -> Self {
        let node = NodeBuilder::new()
            .create::<ServiceType>()
            .expect("Failed to create the fault service node");
        let event_publisher_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME).expect("Failed to create the fault service name");
        let event_publisher_service = node
            .service_builder(&event_publisher_service_name)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .expect("Failed to create the hash transmitter service");
        let publisher = event_publisher_service
            .publisher_builder()
            .create()
            .expect("Failed to create the hash transmitter client");

        Self {
            sink_receiver,
            diagnostic_publisher: Some(publisher),
        }
    }

    fn publish_event(&self, event: DiagnosticEvent) -> Result<(), SinkError> {
        let sample = self
            .diagnostic_publisher
            .as_ref()
            .unwrap()
            .loan_uninit()
            .map_err(|_| SinkError::TransportDown)?;
        let sample = sample.write_payload(event);
        match sample.send().map_err(|_| SinkError::TransportDown) {
            Ok(_) => {
                debug!("Event successfully sent!");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn run(&self) {
        while let Ok(msg) = self.sink_receiver.recv() {
            match msg {
                WorkerMsg::Start(parent) => {
                    debug!("Diag IPC worker running");
                    parent.unpark();
                }
                WorkerMsg::Event { event } => {
                    self.publish_event(*event).unwrap_or_else(|_| error!("publish_event failed"));
                }
                WorkerMsg::Exit => {
                    info!("FaultMgrClient worker ends");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(miri))]
mod tests {
    use super::*;
    use crate::fault_manager_sink::WorkerMsg;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_fault_sink_start_and_exit_with_timeout() {
        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        let fault_sink = IpcWorker::new(rx);
        let handle = thread::spawn(move || fault_sink.run());

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
}
