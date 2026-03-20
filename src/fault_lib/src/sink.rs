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

use common::{fault::FaultRecord, sink_error::SinkError, types::DiagnosticEvent};
#[cfg(test)]
use mockall::automock;

// Boundary traits for anything that has side-effects (logging + IPC).

/// Hook for logging/observability of fault reporting.
///
/// Called after each publish attempt with success or error context,
/// enabling applications to mirror fault events into their preferred
/// logging stack (DLT, syslog, tracing, etc.).
///
/// Default implementation: [`NoOpLogHook`] (zero overhead).
pub trait LogHook: Send + Sync + 'static {
    /// Called after successful publish to sink.
    fn on_publish(&self, record: &FaultRecord);
    /// Called when publish to sink fails.
    fn on_error(&self, record: &FaultRecord, error: &SinkError);
}

/// Default `LogHook` that does nothing. Zero overhead.
pub struct NoOpLogHook;

impl LogHook for NoOpLogHook {
    #[inline]
    fn on_publish(&self, _record: &FaultRecord) {}
    #[inline]
    fn on_error(&self, _record: &FaultRecord, _error: &SinkError) {}
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
/// # Errors
///
/// All methods return [`SinkError`] on transport or configuration failures.
pub trait FaultSinkApi: Send + Sync + 'static {
    /// Enqueue a record for delivery to the Diagnostic Fault Manager.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the record cannot be enqueued.
    fn publish(&self, path: &str, record: FaultRecord) -> Result<(), SinkError>;
    /// Check that the fault catalog is agreed upon with the DFM.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the hash check cannot be performed.
    fn check_fault_catalog(&self) -> Result<bool, SinkError>;

    /// Send a raw diagnostic event to the DFM.
    ///
    /// Used for non-fault events such as enabling condition registration
    /// and status changes. Default implementation is a no-op that returns
    /// `Ok(())`, suitable for test sinks that don't need IPC.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the event cannot be sent.
    #[allow(clippy::used_underscore_binding)]
    fn send_event(&self, _event: DiagnosticEvent) -> Result<(), SinkError> {
        Ok(())
    }
}
