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
use common::ipc_service_name::{DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME, DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME};
use common::ipc_service_type::ServiceType;
use common::sink_error::SinkError;
use common::types::DiagnosticEvent;

use crate::fault_record_processor::FaultRecordProcessor;
use crate::sovd_fault_storage::SovdFaultStateStorage;
use iceoryx2::node::NodeBuilder;
use iceoryx2::port::publisher::Publisher;
use iceoryx2::port::subscriber::Subscriber;
use iceoryx2::prelude::{Node, NodeName, ServiceName};
use log::info;
use std::time::Duration;

const DIAGNOSTIC_FAULT_MANAGER_LISTENER_NODE_NAME: &str = "fault_listener_node";
const DIAGNOSTIC_FAULT_MANAGER_LISTENER_CYCLE_TIME: Duration = Duration::from_millis(10);

pub struct FaultLibCommunicator {
    catalog_hash_response_publisher: Publisher<ServiceType, bool, ()>,
    diagnostic_event_subscriber: Subscriber<ServiceType, DiagnosticEvent, ()>,
    node: Node<ServiceType>,
}

impl FaultLibCommunicator {
    pub fn new() -> Self {
        let node_name = NodeName::new(DIAGNOSTIC_FAULT_MANAGER_LISTENER_NODE_NAME).unwrap();
        let node = NodeBuilder::new()
            .name(&node_name)
            .create::<ServiceType>()
            .expect("Failed to create listener node");

        let diagnostic_event_subscriber_service_name = ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME).unwrap();
        let diagnostic_event_subscriber_service = node
            .service_builder(&diagnostic_event_subscriber_service_name)
            .publish_subscribe::<DiagnosticEvent>()
            .open_or_create()
            .expect("Failed to create the event listener service");
        let diagnostic_event_subscriber = diagnostic_event_subscriber_service
            .subscriber_builder()
            .create()
            .expect("Failed to create subscriber");

        let hash_response_service_name =
            ServiceName::new(DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME).expect("Failed to create the fault service name");
        let hash_response_service = node
            .service_builder(&hash_response_service_name)
            .publish_subscribe::<bool>()
            .open_or_create()
            .expect("Failed to create the hash transmitter service");
        let catalog_hash_response_publisher = hash_response_service
            .publisher_builder()
            .create()
            .expect("Failed to create the hash transmitter client");

        FaultLibCommunicator {
            diagnostic_event_subscriber,
            catalog_hash_response_publisher,
            node,
        }
    }

    fn publish_catalog_hash_response(&self, hash_response: bool) -> Result<(), SinkError> {
        let sample = self.catalog_hash_response_publisher.loan_uninit().map_err(|_| SinkError::TransportDown)?;
        let sample = sample.write_payload(hash_response);
        match sample.send().map_err(|_| SinkError::TransportDown) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn run<S: SovdFaultStateStorage>(&self, mut processor: FaultRecordProcessor<S>) {
        info!("FaultLibCommunicator listening...");
        while self.node.wait(DIAGNOSTIC_FAULT_MANAGER_LISTENER_CYCLE_TIME).is_ok() {
            while let Some(sample) = self.diagnostic_event_subscriber.receive().unwrap() {
                match sample.payload() {
                    DiagnosticEvent::Fault((path, fault)) => {
                        info!("Received new fault ID: {:?}", fault.id);
                        processor.process_record(path, fault);
                    }
                    DiagnosticEvent::Hash((path, hash_sum)) => {
                        let result = processor.check_hash_sum(path, hash_sum);
                        info!("Received hash: {:?}", hash_sum);
                        self.publish_catalog_hash_response(result).unwrap();
                    }
                }
            }
        }
        info!("FaultLibCommunicator stopped");
    }
}
