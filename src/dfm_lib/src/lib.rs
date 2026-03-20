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

//! Diagnostic Fault Manager (DFM) library.
//!
//! `dfm_lib` implements the central fault management logic that receives
//! fault reports from distributed `fault_lib` reporters via IPC and
//! applies policies such as debouncing, aging, and lifecycle transitions.
//!
//! ## Key components
//!
//! | Module | Responsibility |
//! |--------|----------------|
//! | [`diagnostic_fault_manager`] | Top-level orchestrator that wires sub-components together |
//! | [`transport`] | [`DfmTransport`](transport::DfmTransport) trait — pluggable IPC abstraction |
//! | [`fault_record_processor`] | Incoming record handling: debounce, hash verification, lifecycle transitions |
//! | [`aging_manager`] | Evaluates aging/reset policies (operation-cycle or time-based) |
//! | [`operation_cycle`] | Tracks named operation cycles (ignition, drive, power) |
//! | [`fault_catalog_registry`] | In-memory registry of catalogs received during handshake |
//! | [`enabling_condition_registry`] | Tracks enabling-condition statuses and broadcasts changes |
//! | [`sovd_fault_manager`] | SOVD-compliant query/clear API for external diagnostic tools |
//! | [`sovd_fault_storage`] | Persistent storage for SOVD fault state (backed by `rust_kvs`) |
//! | [`query_api`] | [`DfmQueryApi`] trait abstracting SOVD query/clear for consumers |
//! | [`query_ipc`] | [`Iceoryx2DfmQuery`] - IPC client implementing `DfmQueryApi` via iceoryx2 |
//! | [`query_server`] | [`DfmQueryServer`](query_server::DfmQueryServer) - DFM-side request handler |
//! | [`query_conversion`] | Bidirectional `SovdFault` <-> `IpcSovdFault` conversion |

extern crate alloc;

pub mod aging_manager;
pub mod diagnostic_fault_manager;
pub mod enabling_condition_registry;
pub mod fault_catalog_registry;
pub(crate) mod fault_lib_communicator;
pub mod fault_record_processor;
pub mod operation_cycle;
pub mod query_api;
pub(crate) mod query_conversion;
pub mod query_ipc;
pub(crate) mod query_server;
pub mod sovd_fault_manager;
pub mod sovd_fault_storage;
pub mod transport;

// Re-export key types for convenience
pub use aging_manager::{AgingManager, AgingState};
pub use enabling_condition_registry::EnablingConditionRegistry;
pub use fault_lib_communicator::{
    DfmLoopExtensions, Iceoryx2Transport, TransportInitError, run_dfm_loop,
};
pub use operation_cycle::{
    CycleEventType, CycleSource, ManualCycleProvider, OperationCycleEvent, OperationCycleProvider,
    OperationCycleTracker,
};
pub use query_api::{DfmQueryApi, DirectDfmQuery};
pub use query_ipc::Iceoryx2DfmQuery;
pub use transport::DfmTransport;

#[cfg(test)]
mod dfm_test_utils;
