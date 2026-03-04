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

//! Shared types and utilities for the fault-lib ecosystem.
//!
//! This crate provides the foundational types used by both the reporter side
//! (`fault_lib`) and the Diagnostic Fault Manager side (`dfm_lib`):
//!
//! - **Fault model** — [`FaultId`], [`FaultDescriptor`](fault::FaultDescriptor),
//!   [`FaultRecord`](fault::FaultRecord), severity levels, lifecycle stages,
//!   and compliance tags (see [`fault`]).
//! - **Catalog** — [`FaultCatalog`](catalog::FaultCatalog) and its builder for
//!   loading fault definitions from JSON or code.
//! - **Debounce** — IPC-safe duration type and debounce strategies
//!   (`CountWithinWindow`, `HoldTime`, `EdgeWithCooldown`) in [`debounce`].
//! - **Configuration** — reset/aging policies and per-report options
//!   ([`config`]).
//! - **IPC plumbing** — fixed-size string types, service names, and the
//!   `DiagnosticEvent` envelope shared over iceoryx2.
//!
//! All public `#[repr(C)]` types in this crate implement `ZeroCopySend` so
//! they can be transferred through iceoryx2 shared-memory channels.

#![warn(missing_docs)]

extern crate alloc;

/// Fault catalog construction and management.
pub mod catalog;
/// Reset/aging policies and per-report runtime options.
pub mod config;
/// Debounce strategies and IPC-safe duration type.
pub mod debounce;
/// Enabling condition types for fault detection gating.
pub mod enabling_condition;
/// Core fault model: IDs, descriptors, records, severities, lifecycle.
pub mod fault;
/// Source identification types for fleet-wide fault attribution.
pub mod ids;
/// Well-known iceoryx2 service name constants.
pub mod ipc_service_name;
/// IPC service type alias for iceoryx2.
pub mod ipc_service_type;
/// IPC wire types for DFM query/clear request-response protocol.
pub mod query_protocol;
/// Error types returned by fault sink implementations.
pub mod sink_error;
/// Fixed-size string types, metadata vectors, and the `DiagnosticEvent` envelope.
pub mod types;

pub use config::{ReportOptions, ResetPolicy};
pub use enabling_condition::{EnablingConditionNotification, EnablingConditionStatus};
pub use fault::FaultId;
pub use ids::SourceId;
pub use types::{to_static_long_string, to_static_short_string};
