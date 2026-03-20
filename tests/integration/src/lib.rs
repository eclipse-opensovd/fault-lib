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
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing
    )
)]
//! Integration tests demonstrating the fault-lib ↔ DFM end-to-end flow.
//!
//! These tests exercise the full pipeline without IPC (iceoryx2), using
//! in-process wiring instead:
//!
//! 1. Build a `FaultCatalog` from JSON config
//! 2. Create a `FaultRecordProcessor` (DFM core) with persistent storage
//! 3. Simulate reporter-side record creation
//! 4. Feed records through the processor
//! 5. Query results via `SovdFaultManager`
//!
//! This mirrors a real deployment where fault-lib reporters publish to DFM
//! over IPC, but tests the logic without shared-memory transport.

#[cfg(test)]
mod helpers;
#[cfg(test)]
mod test_boundary_values;
#[cfg(test)]
mod test_concurrent_access;
#[cfg(test)]
mod test_debounce_aging_cycles_ec;
#[cfg(test)]
mod test_error_paths;
#[cfg(test)]
mod test_ipc_query;
#[cfg(test)]
mod test_json_catalog;
#[cfg(test)]
mod test_lifecycle_transitions;
#[cfg(test)]
mod test_multi_catalog;
#[cfg(test)]
mod test_persistent_storage;
#[cfg(test)]
mod test_report_and_query;
