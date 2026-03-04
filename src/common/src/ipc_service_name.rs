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

//! IPC service name constants for iceoryx2 publish-subscribe channels.
//!
//! # Naming convention
//!
//! All service names use a hierarchical slash-separated format:
//!
//! ```text
//! <subsystem>/<channel>[/<qualifier>]
//! ```
//!
//! - **`<subsystem>`** — logical component, e.g. `dfm` (Diagnostic Fault Manager).
//! - **`<channel>`** — communication purpose, e.g. `event`, `enabling_condition`.
//! - **`<qualifier>`** — optional sub-channel, e.g. `hash/response`, `notification`.
//!
//! # Constraints
//!
//! - Must be valid iceoryx2 [`ServiceName`](iceoryx2::prelude::ServiceName) values.
//! - Maximum length is 255 bytes (iceoryx2 limit).
//! - Characters allowed: alphanumeric, `/`, `_`, `-`, `.`.
//! - Must not start or end with `/`.
//!
//! # Examples
//!
//! | Constant | Value | Direction |
//! |----------|-------|-----------|
//! | `DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME` | `dfm/event` | reporter → DFM |
//! | `DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME` | `dfm/event/hash/response` | DFM → reporter |
//! | `ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME` | `dfm/enabling_condition/notification` | DFM → reporters |

/// Iceoryx2 service name for the main diagnostic-event channel (reporter → DFM).
///
/// Reporters publish [`DiagnosticEvent`](crate::types::DiagnosticEvent) messages on this
/// channel. The Diagnostic Fault Manager subscribes and processes them.
pub const DIAGNOSTIC_FAULT_MANAGER_EVENT_SERVICE_NAME: &str = "dfm/event";

/// Iceoryx2 service name for hash-check responses (DFM → reporter).
///
/// After a reporter registers, the DFM replies on this channel with a
/// hash-check response confirming catalog consistency.
pub const DIAGNOSTIC_FAULT_MANAGER_HASH_CHECK_RESPONSE_SERVICE_NAME: &str =
    "dfm/event/hash/response";

/// Iceoryx2 service name for enabling-condition notifications (DFM → reporters).
///
/// The DFM publishes [`EnablingConditionNotification`](crate::EnablingConditionNotification)
/// messages whenever an enabling condition changes state.
pub const ENABLING_CONDITION_NOTIFICATION_SERVICE_NAME: &str =
    "dfm/enabling_condition/notification";

/// Iceoryx2 service name for DFM query request-response (external tool -> DFM).
///
/// Used by the iceoryx2 native request-response API. External diagnostic
/// tools send [`DfmQueryRequest`](crate::query_protocol::DfmQueryRequest)
/// and receive [`DfmQueryResponse`](crate::query_protocol::DfmQueryResponse).
pub const DFM_QUERY_SERVICE_NAME: &str = "dfm/query";
