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

//! DFM-side query request handler.
//!
//! [`DfmQueryServer`] receives [`DfmQueryRequest`]s from external diagnostic
//! tools via iceoryx2 request-response and dispatches them to the local
//! [`SovdFaultManager`]. Called from the DFM event loop via [`poll`](DfmQueryServer::poll).

use common::{
    ipc_service_name::DFM_QUERY_SERVICE_NAME,
    ipc_service_type::ServiceType,
    query_protocol::{DfmQueryError, DfmQueryRequest, DfmQueryResponse, MAX_FAULTS_PER_RESPONSE},
    sink_error::SinkError,
    types::ShortString,
};
use iceoryx2::{port::server::Server, prelude::*};
use iceoryx2_bb_container::vector::Vector;

use crate::{
    query_conversion::{env_data_to_ipc, sovd_fault_to_ipc},
    sovd_fault_manager::{Error as SovdError, SovdFaultManager},
    sovd_fault_storage::SovdFaultStateStorage,
};

/// Server-side handler for DFM query requests.
///
/// Wraps an iceoryx2 `Server` and a reference to the `SovdFaultManager`.
/// Call [`poll`](Self::poll) from the DFM event loop to process pending
/// requests.
pub struct DfmQueryServer<S: SovdFaultStateStorage> {
    server: Server<ServiceType, DfmQueryRequest, (), DfmQueryResponse, ()>,
    sovd_manager: SovdFaultManager<S>,
}

impl<S: SovdFaultStateStorage> DfmQueryServer<S> {
    /// Create a new query server on the given iceoryx2 node.
    ///
    /// Opens (or creates) the `dfm/query` request-response service and
    /// creates a server port.
    ///
    /// # Errors
    ///
    /// Returns `SinkError::TransportDown` if the iceoryx2 service or server
    /// port cannot be created.
    pub fn new(
        node: &Node<ServiceType>,
        sovd_manager: SovdFaultManager<S>,
    ) -> Result<Self, SinkError> {
        let service_name =
            ServiceName::new(DFM_QUERY_SERVICE_NAME).map_err(|_| SinkError::TransportDown)?;
        let service = node
            .service_builder(&service_name)
            .request_response::<DfmQueryRequest, DfmQueryResponse>()
            .open_or_create()
            .map_err(|_| SinkError::TransportDown)?;
        let server = service
            .server_builder()
            .create()
            .map_err(|_| SinkError::TransportDown)?;

        Ok(Self {
            server,
            sovd_manager,
        })
    }

    /// Process all pending query requests (non-blocking).
    ///
    /// Called once per DFM event-loop iteration. Drains all queued requests,
    /// dispatches each to [`SovdFaultManager`], and sends the response back
    /// to the requesting client.
    ///
    /// Errors from individual requests are sent as `DfmQueryResponse::Error`
    /// to the client - they do not propagate to the caller. Only transport-level
    /// failures (send/receive broken) return `Err`.
    pub fn poll(&self) -> Result<(), SinkError> {
        loop {
            let active_request = match self.server.receive() {
                Ok(Some(req)) => req,
                Ok(None) => break, // no more pending requests
                Err(_) => return Err(SinkError::TransportDown),
            };

            let response = self.handle_request(&active_request);
            active_request
                .send_copy(response)
                .map_err(|_| SinkError::TransportDown)?;
        }
        Ok(())
    }

    fn handle_request(&self, request: &DfmQueryRequest) -> DfmQueryResponse {
        match request {
            DfmQueryRequest::GetAllFaults(path) => {
                let path_str = path.to_string();
                match self.sovd_manager.get_all_faults(&path_str) {
                    Ok(faults) => {
                        let total_count = u32::try_from(faults.len()).unwrap_or(u32::MAX);
                        let mut ipc_faults = common::query_protocol::IpcFaultListResponse {
                            faults: iceoryx2_bb_container::vector::StaticVec::new(),
                            total_count,
                        };
                        for fault in faults.iter().take(MAX_FAULTS_PER_RESPONSE) {
                            let _ = ipc_faults.faults.push(sovd_fault_to_ipc(fault));
                        }
                        DfmQueryResponse::FaultList(ipc_faults)
                    }
                    Err(e) => DfmQueryResponse::Error(sovd_error_to_ipc(e)),
                }
            }
            DfmQueryRequest::GetFault(path, fault_code) => {
                let path_str = path.to_string();
                let code_str = fault_code.to_string();
                match self.sovd_manager.get_fault(&path_str, &code_str) {
                    Ok((fault, env)) => DfmQueryResponse::SingleFault(
                        sovd_fault_to_ipc(&fault),
                        env_data_to_ipc(&env),
                    ),
                    Err(e) => DfmQueryResponse::Error(sovd_error_to_ipc(e)),
                }
            }
            DfmQueryRequest::DeleteAllFaults(path) => {
                let path_str = path.to_string();
                match self.sovd_manager.delete_all_faults(&path_str) {
                    Ok(()) => DfmQueryResponse::Ok,
                    Err(e) => DfmQueryResponse::Error(sovd_error_to_ipc(e)),
                }
            }
            DfmQueryRequest::DeleteFault(path, fault_code) => {
                let path_str = path.to_string();
                let code_str = fault_code.to_string();
                match self.sovd_manager.delete_fault(&path_str, &code_str) {
                    Ok(()) => DfmQueryResponse::Ok,
                    Err(e) => DfmQueryResponse::Error(sovd_error_to_ipc(e)),
                }
            }
        }
    }
}

fn sovd_error_to_ipc(e: SovdError) -> DfmQueryError {
    match e {
        SovdError::BadArgument => DfmQueryError::BadArgument,
        SovdError::NotFound => DfmQueryError::NotFound,
        SovdError::Storage(msg) => {
            let truncated = ShortString::from_str_truncated(&msg).unwrap_or_default();
            if msg.len() > truncated.len() {
                tracing::warn!(
                    "IPC error message truncation: input {} bytes -> {} bytes",
                    msg.len(),
                    truncated.len()
                );
            }
            DfmQueryError::StorageError(truncated)
        }
    }
}
