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

//! IPC wire types for DFM query/clear request-response protocol.
//!
//! These types are `#[repr(C)]` + [`ZeroCopySend`] for iceoryx2 shared-memory
//! transport. [`IpcSovdFault`] is the IPC-safe equivalent of
//! `SovdFault` (in dfm_lib) with fixed-size fields.
//!
//! # Lossy conversion
//!
//! `SovdFault` -> `IpcSovdFault` is lossy:
//! - `String` fields truncated to `ShortString` (64B) or `LongString` (128B)
//! - `HashMap<String, String> status` omitted (reconstructable from bool fields)
//! - `Option<String> schema` omitted (not used in runtime query flow)
//! - `Option<bool>` -> `bool` (None -> false)

use crate::types::{LongString, ShortString};
use iceoryx2::prelude::ZeroCopySend;
use iceoryx2_bb_container::vector::StaticVec;

/// Maximum number of faults in a single IPC response.
pub const MAX_FAULTS_PER_RESPONSE: usize = 64;

/// IPC-safe fault representation with fixed-size fields.
///
/// Converted from/to `SovdFault` via conversion functions in `dfm_lib`.
/// See module-level docs for lossy conversion details.
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub struct IpcSovdFault {
    // --- Core identification ---
    /// Fault code (e.g., "0x1001", "`fault_a`"). Truncated to 64B.
    pub code: ShortString,
    /// Human-readable display code. Truncated to 64B.
    pub display_code: ShortString,
    /// Fault scope (e.g., "ecu"). Truncated to 64B.
    pub scope: ShortString,
    /// Fault name. Truncated to 64B.
    pub fault_name: ShortString,
    /// Translation key. Truncated to 64B.
    pub fault_translation_id: ShortString,
    /// Severity level.
    pub severity: u32,

    // --- DTC status flags (flattened from SovdFaultStatus) ---
    /// ISO 14229 status mask byte.
    pub status_mask: u8,
    /// UDS bit 0: testFailed.
    pub test_failed: bool,
    /// UDS bit 1: testFailedThisOperationCycle.
    pub test_failed_this_operation_cycle: bool,
    /// UDS bit 2: pendingDTC.
    pub pending_dtc: bool,
    /// UDS bit 3: confirmedDTC.
    pub confirmed_dtc: bool,
    /// UDS bit 4: testNotCompletedSinceLastClear.
    pub test_not_completed_since_last_clear: bool,
    /// UDS bit 5: testFailedSinceLastClear.
    pub test_failed_since_last_clear: bool,
    /// UDS bit 6: testNotCompletedThisOperationCycle.
    pub test_not_completed_this_operation_cycle: bool,
    /// UDS bit 7: warningIndicatorRequested.
    pub warning_indicator_requested: bool,

    // --- Counters & timestamps ---
    /// Number of occurrences.
    pub occurrence_counter: u32,
    /// Aging cycles since last occurrence.
    pub aging_counter: u32,
    /// Number of heals/resets.
    pub healing_counter: u32,
    /// Unix timestamp (secs) of first occurrence. 0 = not set.
    pub first_occurrence_secs: u64,
    /// Unix timestamp (secs) of last occurrence. 0 = not set.
    pub last_occurrence_secs: u64,

    // --- Optional fields (has_* pattern for Option encoding) ---
    /// Symptom description. Truncated to 128B. Check `has_symptom`.
    pub symptom: LongString,
    /// Whether `symptom` is populated (encodes `Option::Some`).
    pub has_symptom: bool,
    /// Symptom translation key. Check `has_symptom_translation_id`.
    pub symptom_translation_id: ShortString,
    /// Whether `symptom_translation_id` is populated.
    pub has_symptom_translation_id: bool,
}

/// IPC-safe environment data: up to 8 key-value pairs.
pub type IpcEnvData = StaticVec<(ShortString, ShortString), 8>;

/// Response payload for `GetAllFaults`.
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub struct IpcFaultListResponse {
    /// Faults in this response (up to [`MAX_FAULTS_PER_RESPONSE`]).
    pub faults: StaticVec<IpcSovdFault, MAX_FAULTS_PER_RESPONSE>,
    /// Total number of faults in the catalog (may exceed `faults.len()`).
    pub total_count: u32,
}

/// Request variants for the DFM query service.
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub enum DfmQueryRequest {
    /// List all faults for entity at `path`.
    GetAllFaults(LongString),
    /// Get single fault: `(path, fault_code)`.
    GetFault(LongString, ShortString),
    /// Delete all fault state for entity at `path` (removes from storage entirely).
    /// Note: this is SOVD `DeleteFault`, not UDS $14 `ClearDiagnosticInformation`.
    // TODO: Add ClearDtc/ClearSingleDtc variants for ISO 14229 UDS $14 compliance
    DeleteAllFaults(LongString),
    /// Delete single fault state: `(path, fault_code)` (removes from storage entirely).
    /// Note: this is SOVD `DeleteFault`, not UDS $14 `ClearDiagnosticInformation`.
    DeleteFault(LongString, ShortString),
}

/// Error variants returned over IPC.
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub enum DfmQueryError {
    /// Invalid path or argument.
    BadArgument,
    /// Fault not found.
    NotFound,
    /// Storage backend error (message truncated to 64B).
    StorageError(ShortString),
}

/// Response variants for the DFM query service.
///
/// Large variant size disparity (`FaultList` vs Ok) is intentional:
/// boxing is not possible for IPC types crossing shared-memory boundaries.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub enum DfmQueryResponse {
    /// List of faults (response to `GetAllFaults`).
    FaultList(IpcFaultListResponse),
    /// Single fault with env data (response to `GetFault`).
    SingleFault(IpcSovdFault, IpcEnvData),
    /// Success (response to `DeleteAllFaults` / `DeleteFault`).
    Ok,
    /// Error response.
    Error(DfmQueryError),
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use iceoryx2_bb_container::vector::Vector;

    #[test]
    fn ipc_sovd_fault_is_zero_copy_send() {
        // Compile-time check: ZeroCopySend is required for iceoryx2 shared memory.
        fn assert_zero_copy_send<T: ZeroCopySend>() {}
        assert_zero_copy_send::<IpcSovdFault>();
        assert_zero_copy_send::<DfmQueryRequest>();
        assert_zero_copy_send::<DfmQueryResponse>();
        assert_zero_copy_send::<IpcFaultListResponse>();
    }

    #[test]
    fn ipc_fault_list_response_capacity() {
        let response = IpcFaultListResponse {
            faults: StaticVec::new(),
            total_count: 100,
        };
        assert_eq!(response.faults.capacity(), MAX_FAULTS_PER_RESPONSE);
        assert_eq!(response.faults.len(), 0);
        assert_eq!(response.total_count, 100);
    }

    #[test]
    fn dfm_query_request_variants_constructible() {
        let _req1 = DfmQueryRequest::GetAllFaults(LongString::from_str_truncated("hvac").unwrap());
        let _req2 = DfmQueryRequest::GetFault(
            LongString::from_str_truncated("hvac").unwrap(),
            ShortString::try_from("0x7001").unwrap(),
        );
        let _req3 =
            DfmQueryRequest::DeleteAllFaults(LongString::from_str_truncated("hvac").unwrap());
        let _req4 = DfmQueryRequest::DeleteFault(
            LongString::from_str_truncated("hvac").unwrap(),
            ShortString::try_from("0x7001").unwrap(),
        );
    }

    #[test]
    fn dfm_query_response_variants_constructible() {
        let _ok = DfmQueryResponse::Ok;
        let _err_bad = DfmQueryResponse::Error(DfmQueryError::BadArgument);
        let _err_nf = DfmQueryResponse::Error(DfmQueryError::NotFound);
        let _err_stor = DfmQueryResponse::Error(DfmQueryError::StorageError(
            ShortString::try_from("disk full").unwrap(),
        ));
    }
}
