/*
* Copyright (c) 2025 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
*
* See the NOTICE file(s) distributed with this work for additional
* information regarding copyright ownership.
*
* This program and the accompanying materials are made available under the
* terms of the Apache License Version 2.0 which is available at
* https://www.apache.org/licenses/LICENSE-2.0
*
* SPDX-License-Identifier: Apache-2.0
*/

use crate::model::FaultRecord;

// Boundary traits for anything that has side-effects (logging + IPC).

/// Hook to ensure that reporting a fault additionally results in a log entry.
/// Default impl can forward to log.
pub trait LogHook: Send + Sync + 'static {
    fn on_report(&self, record: &FaultRecord);
}

/// Sink abstracts the transport to the Diagnostic Fault Manager.
/// Implementations can be S-CORE IPC.
#[allow(async_fn_in_trait)]
pub trait FaultSink: Send + Sync + 'static {
    /// Publish a record.
    fn publish(&self, record: &FaultRecord) -> Result<(), SinkError>;
}

#[derive(thiserror::Error, Debug)]
pub enum SinkError {
    #[error("transport unavailable")]
    TransportDown,
    #[error("rate limited")]
    RateLimited,
    #[error("permission denied")]
    PermissionDenied,
    #[error("invalid descriptor: {0}")]
    BadDescriptor(&'static str),
    #[error("other: {0}")]
    Other(&'static str),
}
