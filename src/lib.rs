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

#![forbid(unsafe_code)] // enforce safe Rust across the crate
#![feature(const_option_ops)] //
#![feature(const_trait_impl)]
// The public surface collects the building blocks for reporters, descriptors,
// and sinks so callers can just `use fault_lib::*` and go.
pub mod api;
pub mod catalog;
pub mod config;
pub mod ids;
pub mod model;
pub mod sink;
pub mod utils;

// Re-export the main user-facing pieces, this keeps the crate ergonomic without
// forcing consumers to dig through modules.
pub use api::{FaultApi, Reporter};
pub use catalog::FaultCatalog;
pub use config::{DebouncePolicy, ReportOptions, ReporterConfig, ResetPolicy};
pub use ids::{FaultId, SourceId};
pub use model::{
    ComplianceTag, FaultDescriptor, FaultLifecycleStage, FaultRecord, FaultSeverity, FaultType,
    KeyValue, LifecyclePhase,
};
pub use sink::{FaultSink, LogHook};
