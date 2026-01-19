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
#![forbid(unsafe_code)] // enforce safe Rust across the crate
// The public surface collects the building blocks for reporters, descriptors,
// and sinks so callers can just `use fault_lib::*` and go.
pub mod api;
pub mod catalog;
pub mod reporter;
pub mod sink;

mod fault_manager_sink;
mod ipc_worker;

pub use api::FaultApi;
// Re-export the main user-facing pieces, this keeps the crate ergonomic without
// forcing consumers to dig through modules.
// pub use api::{FaultApi, Reporter};
// pub use catalog::FaultCatalog;
pub use sink::{FaultSinkApi, LogHook};

pub mod utils;

pub mod test_utils;
