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

use common::fault::FaultRecord;
use common::sink_error::SinkError;

#[allow(unused_imports)]
use mockall::automock;

// Boundary traits for anything that has side-effects (logging + IPC).

/// Hook to ensure that reporting a fault additionally results in a log entry.
/// Default impl can forward to log.
pub trait LogHook: Send + Sync + 'static {
    fn on_report(&self, record: &FaultRecord);
}

/// Sink abstracts the transport to the Diagnostic Fault Manager.
///
/// Non-blocking contract:
/// - MUST return quickly (enqueue only) without waiting on IPC/network/disk.
/// - SHOULD avoid allocating excessively or performing locking that can contend with hot paths.
/// - Backpressure and retry are internal; caller only gets enqueue success/failure.
/// - Lifetime: installed once in `FaultApi::new` and lives for the duration of the process.
///
/// Implementations can be S-CORE IPC.
#[cfg_attr(test, automock)]
pub trait FaultSinkApi: Send + Sync + 'static {
    /// Enqueue a record for delivery to the Diagnostic Fault Manager.
    fn publish(&self, path: &str, record: FaultRecord) -> Result<(), SinkError>;
    fn check_fault_catalog(&self) -> Result<bool, SinkError>;
}
