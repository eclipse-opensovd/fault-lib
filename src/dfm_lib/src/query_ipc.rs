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

//! IPC client implementation of [`DfmQueryApi`].
//!
//! [`Iceoryx2DfmQuery`] sends [`DfmQueryRequest`]s to the DFM process via
//! iceoryx2 native request-response and converts the [`DfmQueryResponse`]
//! back to `SovdFault` / `SovdEnvData`.

use alloc::{format, string::String, vec::Vec};
use core::time::Duration;

use common::{
    ipc_service_name::DFM_QUERY_SERVICE_NAME,
    ipc_service_type::ServiceType,
    query_protocol::{DfmQueryError, DfmQueryRequest, DfmQueryResponse},
    types::{LongString, ShortString},
};
use iceoryx2::{port::client::Client, prelude::*};

use crate::{
    query_api::DfmQueryApi,
    query_conversion::{ipc_env_data_to_sovd, ipc_fault_to_sovd},
    sovd_fault_manager::{Error, SovdEnvData, SovdFault},
};

/// Default timeout for waiting for a response from the DFM.
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(1);

/// IPC-based [`DfmQueryApi`] client.
///
/// Connects to the DFM's `dfm/query` request-response service and sends
/// queries/clears via iceoryx2 shared memory. The DFM must have a
/// [`DfmQueryServer`](crate::query_server::DfmQueryServer) running.
pub struct Iceoryx2DfmQuery {
    client: Client<ServiceType, DfmQueryRequest, (), DfmQueryResponse, ()>,
    node: Node<ServiceType>,
    timeout: Duration,
}

impl Iceoryx2DfmQuery {
    /// Create a new IPC query client.
    ///
    /// Connects to the `dfm/query` service. The service must already exist
    /// (created by the DFM's `DfmQueryServer`).
    ///
    /// # Errors
    ///
    /// Returns `Error::Storage` if the iceoryx2 service or client port
    /// cannot be opened.
    pub fn new() -> Result<Self, Error> {
        Self::with_timeout(DEFAULT_RESPONSE_TIMEOUT)
    }

    /// Create a new IPC query client with a custom response timeout.
    ///
    /// # Errors
    ///
    /// Returns `Error::Storage` if the iceoryx2 service or client port
    /// cannot be opened.
    pub fn with_timeout(timeout: Duration) -> Result<Self, Error> {
        let node = NodeBuilder::new()
            .create::<ServiceType>()
            .map_err(|e| Error::Storage(format!("failed to create node: {e:?}")))?;

        let service_name = ServiceName::new(DFM_QUERY_SERVICE_NAME)
            .map_err(|e| Error::Storage(format!("invalid service name: {e:?}")))?;

        let service = node
            .service_builder(&service_name)
            .request_response::<DfmQueryRequest, DfmQueryResponse>()
            .open_or_create()
            .map_err(|e| Error::Storage(format!("failed to open query service: {e:?}")))?;

        let client = service
            .client_builder()
            .create()
            .map_err(|e| Error::Storage(format!("failed to create client: {e:?}")))?;

        Ok(Self {
            client,
            node,
            timeout,
        })
    }

    /// Send a request and wait for the response (blocking with timeout).
    #[allow(clippy::arithmetic_side_effects)]
    fn request(&self, req: DfmQueryRequest) -> Result<DfmQueryResponse, Error> {
        let pending = self
            .client
            .send_copy(req)
            .map_err(|e| Error::Storage(format!("send failed: {e:?}")))?;

        // Poll for response with timeout
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            if let Some(response) = pending
                .receive()
                .map_err(|e| Error::Storage(format!("receive failed: {e:?}")))?
            {
                return Ok(response.payload().clone());
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Storage(String::from("query timeout")));
            }
            // Brief sleep to avoid busy-waiting; DFM cycle is ~10ms
            let _ = self.node.wait(Duration::from_millis(1));
        }
    }
}

impl DfmQueryApi for Iceoryx2DfmQuery {
    fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        let req =
            DfmQueryRequest::GetAllFaults(LongString::from_str_truncated(path).unwrap_or_default());
        match self.request(req)? {
            DfmQueryResponse::FaultList(list) => {
                Ok(list.faults.iter().map(ipc_fault_to_sovd).collect())
            }
            DfmQueryResponse::Error(e) => Err(ipc_error_to_sovd(&e)),
            other => Err(Error::Storage(format!("unexpected response: {other:?}"))),
        }
    }

    fn get_fault(&self, path: &str, fault_code: &str) -> Result<(SovdFault, SovdEnvData), Error> {
        let req = DfmQueryRequest::GetFault(
            LongString::from_str_truncated(path).unwrap_or_default(),
            ShortString::from_str_truncated(fault_code).unwrap_or_default(),
        );
        match self.request(req)? {
            DfmQueryResponse::SingleFault(ipc_fault, ipc_env) => Ok((
                ipc_fault_to_sovd(&ipc_fault),
                ipc_env_data_to_sovd(&ipc_env),
            )),
            DfmQueryResponse::Error(e) => Err(ipc_error_to_sovd(&e)),
            other => Err(Error::Storage(format!("unexpected response: {other:?}"))),
        }
    }

    fn delete_all_faults(&self, path: &str) -> Result<(), Error> {
        let req = DfmQueryRequest::DeleteAllFaults(
            LongString::from_str_truncated(path).unwrap_or_default(),
        );
        match self.request(req)? {
            DfmQueryResponse::Ok => Ok(()),
            DfmQueryResponse::Error(e) => Err(ipc_error_to_sovd(&e)),
            other => Err(Error::Storage(format!("unexpected response: {other:?}"))),
        }
    }

    fn delete_fault(&self, path: &str, fault_code: &str) -> Result<(), Error> {
        let req = DfmQueryRequest::DeleteFault(
            LongString::from_str_truncated(path).unwrap_or_default(),
            ShortString::from_str_truncated(fault_code).unwrap_or_default(),
        );
        match self.request(req)? {
            DfmQueryResponse::Ok => Ok(()),
            DfmQueryResponse::Error(e) => Err(ipc_error_to_sovd(&e)),
            other => Err(Error::Storage(format!("unexpected response: {other:?}"))),
        }
    }
}

fn ipc_error_to_sovd(e: &DfmQueryError) -> Error {
    match e {
        DfmQueryError::BadArgument => Error::BadArgument,
        DfmQueryError::NotFound => Error::NotFound,
        DfmQueryError::StorageError(msg) => Error::Storage(msg.to_string()),
    }
}
