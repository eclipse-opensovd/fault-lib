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

//! Transport abstraction for DFM ↔ `FaultLib` communication.
//!
//! [`DfmTransport`] decouples the DFM run-loop from the concrete IPC
//! mechanism (iceoryx2). Implementations can use shared-memory IPC,
//! in-process channels, or any other messaging backend.
//!
//! The default production implementation is
//! [`Iceoryx2Transport`](crate::fault_lib_communicator::Iceoryx2Transport),
//! which uses iceoryx2 zero-copy shared memory.

use common::enabling_condition::EnablingConditionNotification;
use common::sink_error::SinkError;
use common::types::DiagnosticEvent;
use core::time::Duration;

/// Abstraction over the DFM-side IPC transport.
///
/// The DFM run-loop calls these methods each iteration to receive events
/// from reporter applications, publish hash-check responses, and broadcast
/// enabling-condition notifications.
///
/// # Implementing a custom transport
///
/// ```rust,ignore
/// use dfm_lib::transport::DfmTransport;
///
/// struct MyTransport { /* ... */ }
///
/// impl DfmTransport for MyTransport {
///     fn receive_event(&self) -> Result<Option<DiagnosticEvent>, SinkError> { /* ... */ }
///     fn publish_hash_response(&self, response: bool) -> Result<(), SinkError> { /* ... */ }
///     fn publish_ec_notification(&self, notification: EnablingConditionNotification) -> Result<(), SinkError> { /* ... */ }
///     fn wait(&self, timeout: Duration) -> Result<bool, SinkError> { /* ... */ }
/// }
/// ```
/// # Errors
///
/// All methods return [`SinkError`] on transport failures.
pub trait DfmTransport: Send + 'static {
    /// Receive the next diagnostic event, if available.
    ///
    /// Returns `Ok(None)` when no events are pending (non-blocking).
    /// Returns `Ok(Some(event))` for each queued event.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the underlying transport fails.
    fn receive_event(&self) -> Result<Option<DiagnosticEvent>, SinkError>;

    /// Publish a catalog hash-check response back to reporters.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the response cannot be published.
    fn publish_hash_response(&self, response: bool) -> Result<(), SinkError>;

    /// Broadcast an enabling-condition status notification to all `FaultLib`
    /// subscribers.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the notification cannot be broadcast.
    fn publish_ec_notification(
        &self,
        notification: EnablingConditionNotification,
    ) -> Result<(), SinkError>;

    /// Wait/sleep for one DFM cycle iteration.
    ///
    /// Returns `Ok(true)` if the node is still alive and the loop should
    /// continue. Returns `Ok(false)` if the node died and the loop should
    /// exit.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the wait operation fails.
    fn wait(&self, timeout: Duration) -> Result<bool, SinkError>;
}
