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

//! Reporter-side fault library for the `OpenSOVD` / S-CORE ecosystem.
//!
//! `fault_lib` provides the application-facing API for reporting faults to
//! the Diagnostic Fault Manager (DFM) over IPC.  The main workflow is:
//!
//! 1. Build a [`FaultCatalog`](common::catalog::FaultCatalog) from JSON or
//!    code.
//! 2. Initialise the global [`FaultApi`] singleton with a transport sink and
//!    an optional log hook.
//! 3. Create one [`Reporter`](reporter::Reporter) per fault ID — each
//!    reporter binds static descriptor data once.
//! 4. At runtime, call `reporter.create_record()`, mutate the
//!    [`FaultRecord`](common::fault::FaultRecord), and `reporter.publish()`
//!    to enqueue it (non-blocking).
//!
//! ## Feature highlights
//!
//! - **Enabling conditions** — [`EnablingCondition`] / [`FaultMonitor`] gate
//!   fault detection based on DFM-broadcast status changes.
//! - **Reporter-side debounce** — descriptor-declared policies filter noisy
//!   events before they hit IPC.
//! - **Pluggable transport** — implement [`FaultSinkApi`] (or use the built-in
//!   iceoryx2 sink) and [`LogHook`] for observability.
//!
//! See [`enabling_condition`] for the enabling-condition subsystem.

extern crate alloc;
// The public surface collects the building blocks for reporters, descriptors,
// and sinks so callers can just `use fault_lib::*` and go.
pub mod api;
pub mod catalog;
pub mod enabling_condition;
pub mod reporter;
pub mod sink;

mod fault_manager_sink;
mod ipc_worker;

pub use api::FaultApi;
// Re-export the main user-facing pieces, this keeps the crate ergonomic without
// forcing consumers to dig through modules.
// pub use api::{FaultApi, Reporter};
// pub use catalog::FaultCatalog;
pub use enabling_condition::{
    EnablingCondition, EnablingConditionCallback, EnablingConditionError, FaultMonitor,
};
pub use sink::{FaultSinkApi, LogHook, NoOpLogHook};

pub mod utils;

#[cfg(any(test, feature = "testutils"))]
pub mod test_utils;
