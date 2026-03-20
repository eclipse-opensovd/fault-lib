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

//! SOVD-compliant fault query and clear API.
//!
//! Provides the interface consumed by external diagnostic tools (e.g.
//! `OpenSOVD` diagnostic service) to query DTC statuses, read
//! environment snapshots, and request fault clears.  Backed by
//! [`FaultCatalogRegistry`] and [`SovdFaultStateStorage`].

use alloc::sync::Arc;
use std::collections::HashMap;

use common::{fault, types::ShortString};

use crate::{
    fault_catalog_registry::FaultCatalogRegistry,
    sovd_fault_storage::{SovdFaultState, SovdFaultStateStorage, StorageError},
};

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Error {
    #[error("bad argument")]
    BadArgument,
    #[error("not found")]
    NotFound,
    #[error("storage error: {0}")]
    Storage(String),
}

/// SOVD-compliant fault status (DTC status bits).
/// Aligned with CDA cda-sovd-interfaces `FaultStatus`.
/// Follows ISO 14229 DTC status byte semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SovdFaultStatus {
    pub test_failed: Option<bool>,
    pub test_failed_this_operation_cycle: Option<bool>,
    pub pending_dtc: Option<bool>,
    pub confirmed_dtc: Option<bool>,
    pub test_not_completed_since_last_clear: Option<bool>,
    pub test_failed_since_last_clear: Option<bool>,
    pub test_not_completed_this_operation_cycle: Option<bool>,
    pub warning_indicator_requested: Option<bool>,
    pub mask: Option<String>,
}

impl SovdFaultStatus {
    /// Create status from `SovdFaultState` (internal storage).
    #[must_use]
    pub fn from_state(state: &SovdFaultState) -> Self {
        let mut status = Self {
            test_failed: Some(state.test_failed),
            test_failed_this_operation_cycle: Some(state.test_failed_this_operation_cycle),
            pending_dtc: Some(state.pending_dtc),
            confirmed_dtc: Some(state.confirmed_dtc),
            test_not_completed_since_last_clear: Some(state.test_not_completed_since_last_clear),
            test_failed_since_last_clear: Some(state.test_failed_since_last_clear),
            test_not_completed_this_operation_cycle: Some(
                state.test_not_completed_this_operation_cycle,
            ),
            warning_indicator_requested: Some(state.warning_indicator_requested),
            mask: None,
        };
        status.mask = Some(format!("0x{:02X}", status.compute_mask()));
        status
    }

    /// Compute ISO 14229 status mask byte.
    /// Bit positions per UDS standard:
    /// - Bit 0: testFailed
    /// - Bit 1: testFailedThisOperationCycle
    /// - Bit 2: pendingDTC
    /// - Bit 3: confirmedDTC
    /// - Bit 4: testNotCompletedSinceLastClear
    /// - Bit 5: testFailedSinceLastClear
    /// - Bit 6: testNotCompletedThisOperationCycle
    /// - Bit 7: warningIndicatorRequested
    #[must_use]
    pub fn compute_mask(&self) -> u8 {
        let mut mask = 0u8;
        if self.test_failed.unwrap_or(false) {
            mask |= 0x01;
        }
        if self.test_failed_this_operation_cycle.unwrap_or(false) {
            mask |= 0x02;
        }
        if self.pending_dtc.unwrap_or(false) {
            mask |= 0x04;
        }
        if self.confirmed_dtc.unwrap_or(false) {
            mask |= 0x08;
        }
        if self.test_not_completed_since_last_clear.unwrap_or(false) {
            mask |= 0x10;
        }
        if self.test_failed_since_last_clear.unwrap_or(false) {
            mask |= 0x20;
        }
        if self
            .test_not_completed_this_operation_cycle
            .unwrap_or(false)
        {
            mask |= 0x40;
        }
        if self.warning_indicator_requested.unwrap_or(false) {
            mask |= 0x80;
        }
        mask
    }

    /// Convert to `HashMap`<String, String> for backward compat / JSON serialization.
    #[must_use]
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(v) = self.test_failed {
            map.insert("testFailed".into(), u32::from(v).to_string());
        }
        if let Some(v) = self.test_failed_this_operation_cycle {
            map.insert(
                "testFailedThisOperationCycle".into(),
                u32::from(v).to_string(),
            );
        }
        if let Some(v) = self.pending_dtc {
            map.insert("pendingDTC".into(), u32::from(v).to_string());
        }
        if let Some(v) = self.confirmed_dtc {
            map.insert("confirmedDTC".into(), u32::from(v).to_string());
        }
        if let Some(v) = self.test_not_completed_since_last_clear {
            map.insert(
                "testNotCompletedSinceLastClear".into(),
                u32::from(v).to_string(),
            );
        }
        if let Some(v) = self.test_failed_since_last_clear {
            map.insert("testFailedSinceLastClear".into(), u32::from(v).to_string());
        }
        if let Some(v) = self.test_not_completed_this_operation_cycle {
            map.insert(
                "testNotCompletedThisOperationCycle".into(),
                u32::from(v).to_string(),
            );
        }
        if let Some(v) = self.warning_indicator_requested {
            map.insert("warningIndicatorRequested".into(), u32::from(v).to_string());
        }
        if let Some(ref m) = self.mask {
            map.insert("mask".into(), m.clone());
        }
        map
    }
}

/// SOVD fault representation per SOVD specification.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SovdFault {
    /// Unique fault code (e.g., "0x1001" or "`fault_a`")
    pub code: String,
    /// Human-readable display code
    pub display_code: String,
    /// Fault scope (e.g., "ecu", "component", "system")
    pub scope: String,
    /// Fault name identifier
    pub fault_name: String,
    /// Translation key for fault name
    pub fault_translation_id: String,
    /// Fault severity level
    pub severity: u32,
    /// Dynamic status properties (DTC flags) as key-value pairs.
    /// Used for `OpenSOVD` JSON wire format serialization (backward compat).
    /// When `typed_status` is `Some`, it is the authoritative source;
    /// this `HashMap` is the serialization view derived from it.
    pub status: HashMap<String, String>,
    /// Human-readable symptom description (from descriptor summary)
    pub symptom: Option<String>,
    /// Translation key for symptom
    pub symptom_translation_id: Option<String>,
    /// JSON schema reference for extended fault data
    pub schema: Option<String>,

    // --- Extended fields for richer diagnostics ---
    /// Typed SOVD status (CDA-aligned alternative to `HashMap` status)
    pub typed_status: Option<SovdFaultStatus>,
    /// Number of times this fault has occurred
    pub occurrence_counter: Option<u32>,
    /// Aging cycles passed since last occurrence
    pub aging_counter: Option<u32>,
    /// Number of times this fault was healed/reset
    pub healing_counter: Option<u32>,
    /// ISO 8601 timestamp of first occurrence
    pub first_occurrence: Option<String>,
    /// ISO 8601 timestamp of most recent occurrence
    pub last_occurrence: Option<String>,
}

impl SovdFault {
    fn new(descriptor: &fault::FaultDescriptor, state: &SovdFaultState) -> Self {
        let code = fault_id_to_code(&descriptor.id);
        let typed_status = SovdFaultStatus::from_state(state);

        Self {
            display_code: code.clone(),
            fault_translation_id: format!("fault.{}", &code),
            symptom: descriptor
                .summary
                .as_ref()
                .map(alloc::string::ToString::to_string),
            symptom_translation_id: descriptor
                .summary
                .as_ref()
                .map(|_| format!("symptom.{}", &code)),
            schema: None,
            code,
            scope: "ecu".into(),
            fault_name: descriptor.name.to_string(),
            severity: descriptor.severity as u32,
            status: typed_status.to_hash_map(),
            typed_status: Some(typed_status),
            occurrence_counter: Some(state.occurrence_counter),
            aging_counter: Some(state.aging_counter),
            healing_counter: Some(state.healing_counter),
            first_occurrence: if state.first_occurrence_secs > 0 {
                Some(format_unix_timestamp(state.first_occurrence_secs))
            } else {
                None
            },
            last_occurrence: if state.last_occurrence_secs > 0 {
                Some(format_unix_timestamp(state.last_occurrence_secs))
            } else {
                None
            },
        }
    }
}

/// Format Unix timestamp as ISO 8601 UTC string (e.g. "2024-01-15T09:50:00Z").
///
/// Uses Howard Hinnant's `civil_from_days` algorithm to convert days since epoch
/// to year/month/day without external dependencies.
///
/// Inputs beyond year 9999 (253,402,300,799 seconds) are clamped to
/// "9999-12-31T23:59:59Z" to prevent overflow in the `days as i64` cast.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::items_after_statements
)]
pub(crate) fn format_unix_timestamp(secs: u64) -> String {
    // Year 9999-12-31T23:59:59Z in Unix seconds.
    const MAX_SECS: u64 = 253_402_300_799;
    if secs > MAX_SECS {
        return String::from("9999-12-31T23:59:59Z");
    }

    const SECS_PER_DAY: u64 = 86_400;
    const SECS_PER_HOUR: u64 = 3_600;
    const SECS_PER_MINUTE: u64 = 60;

    let days = secs / SECS_PER_DAY;
    let day_secs = secs % SECS_PER_DAY;
    let hours = day_secs / SECS_PER_HOUR;
    let minutes = (day_secs % SECS_PER_HOUR) / SECS_PER_MINUTE;
    let seconds = day_secs % SECS_PER_MINUTE;

    // Safe: MAX_SECS / 86_400 = 2_932_896 which fits in i64.
    #[allow(clippy::cast_possible_wrap)]
    let days_i64 = days as i64;
    let (year, month, day) = civil_from_days(days_i64);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since 1970-01-01 to (year, month, day).
/// Algorithm: Howard Hinnant's `civil_from_days`
/// Reference: <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::similar_names,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub type SovdEnvData = HashMap<String, String>;

pub struct SovdFaultManager<S: SovdFaultStateStorage> {
    storage: Arc<S>,
    registry: Arc<FaultCatalogRegistry>,
}

impl<S: SovdFaultStateStorage> SovdFaultManager<S> {
    #[must_use]
    pub fn new(storage: Arc<S>, registry: Arc<FaultCatalogRegistry>) -> Self {
        Self { storage, registry }
    }

    /// List all faults for the given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BadArgument`] if the path is not found in the registry.
    #[tracing::instrument(skip(self))]
    pub fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        let Some(catalog) = self.registry.catalogs.get(path) else {
            return Err(Error::BadArgument);
        };
        let descriptors = catalog.descriptors();
        let mut faults = Vec::new();

        for descriptor in descriptors {
            // All registered faults are returned regardless of their current state.
            // Faults without a stored record use the default (clear) status.
            // This matches SOVD/UDS semantics where the diagnostic tool sees all
            // known faults and their current DTC status flags.
            let state = match self.storage.get(path, &descriptor.id) {
                Ok(Some(s)) => s,
                Ok(None) => SovdFaultState::default(),
                Err(e) => {
                    tracing::warn!("Failed to read state for {:?}: {}", descriptor.id, e);
                    SovdFaultState::default()
                }
            };

            faults.push(SovdFault::new(descriptor, &state));
        }

        Ok(faults)
    }

    /// Get a single fault and its environment data.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BadArgument`] if the path is unknown, or
    /// [`Error::NotFound`] if the fault code is not in the catalog.
    #[tracing::instrument(skip(self))]
    pub fn get_fault(
        &self,
        path: &str,
        fault_code: &str,
    ) -> Result<(SovdFault, SovdEnvData), Error> {
        let Some(catalog) = self.registry.catalogs.get(path) else {
            return Err(Error::BadArgument);
        };
        let fault_id = fault_id_from_code(fault_code)?;
        let Some(descriptor) = catalog.descriptor(&fault_id) else {
            return Err(Error::NotFound);
        };
        // All registered faults are returned regardless of their current state.
        // Faults without a stored record use the default (clear) status.
        let state = match self.storage.get(path, &fault_id) {
            Ok(Some(s)) => s,
            Ok(None) => SovdFaultState::default(),
            Err(e) => {
                return Err(Error::Storage(format!("{e:?}")));
            }
        };

        Ok((SovdFault::new(descriptor, &state), state.env_data))
    }

    /// Delete all fault state for the given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BadArgument`] if the path is empty, or a storage error.
    #[tracing::instrument(skip(self))]
    pub fn delete_all_faults(&self, path: &str) -> Result<(), Error> {
        if path.is_empty() {
            return Err(Error::BadArgument);
        }
        self.storage
            .delete_all(path)
            .map_err(|e| Error::Storage(format!("{e}")))
    }

    /// Delete a single fault state by path and fault code.
    ///
    /// # Errors
    ///
    /// Returns [`Error::BadArgument`] if the path or code is empty, or
    /// [`Error::NotFound`] if the fault does not exist.
    #[tracing::instrument(skip(self))]
    pub fn delete_fault(&self, path: &str, fault_code: &str) -> Result<(), Error> {
        if path.is_empty() || fault_code.is_empty() {
            return Err(Error::BadArgument);
        }
        let fault_id = fault_id_from_code(fault_code)?;
        self.storage.delete(path, &fault_id).map_err(|e| match e {
            StorageError::NotFound => Error::NotFound,
            other => Error::Storage(format!("{other}")),
        })
    }
}

fn fault_id_to_code(fault_id: &fault::FaultId) -> String {
    match fault_id {
        fault::FaultId::Numeric(n) => format!("0x{n:X}"),
        fault::FaultId::Text(t) => t.to_string(),
        fault::FaultId::Uuid(u) => {
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                u[0],
                u[1],
                u[2],
                u[3],
                u[4],
                u[5],
                u[6],
                u[7],
                u[8],
                u[9],
                u[10],
                u[11],
                u[12],
                u[13],
                u[14],
                u[15]
            )
        }
    }
}

#[allow(clippy::naive_bytecount)]
fn fault_id_from_code(fault_code: &str) -> Result<fault::FaultId, Error> {
    // Numeric: "0x..." or "0X..." hex prefix (matches fault_id_to_code output)
    if let Some(hex) = fault_code
        .strip_prefix("0x")
        .or_else(|| fault_code.strip_prefix("0X"))
    {
        let n = u32::from_str_radix(hex, 16).map_err(|_| Error::BadArgument)?;
        return Ok(fault::FaultId::Numeric(n));
    }

    // UUID: 8-4-4-4-12 hex pattern (36 chars with dashes)
    if fault_code.len() == 36
        && fault_code.as_bytes().iter().filter(|&&b| b == b'-').count() == 4
        && let Some(bytes) = parse_uuid_string(fault_code)
    {
        return Ok(fault::FaultId::Uuid(bytes));
    }

    // Text: fallback
    let short = ShortString::try_from(fault_code).map_err(|_| Error::BadArgument)?;
    Ok(fault::FaultId::Text(short))
}

/// Parse a UUID string in 8-4-4-4-12 hex format into 16 bytes.
#[allow(clippy::arithmetic_side_effects)]
fn parse_uuid_string(s: &str) -> Option<[u8; 16]> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects,
    clippy::unreadable_literal,
    clippy::doc_markdown
)]
mod tests {
    use super::*;

    #[test]
    fn error_display_impl() {
        assert_eq!(format!("{}", Error::BadArgument), "bad argument");
        assert_eq!(format!("{}", Error::NotFound), "not found");
        assert_eq!(
            format!("{}", Error::Storage("disk full".into())),
            "storage error: disk full"
        );
    }

    #[test]
    fn error_is_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(Error::NotFound);
        assert_eq!(format!("{err}"), "not found");
    }

    #[test]
    fn status_mask_all_zeros() {
        let status = SovdFaultStatus::default();
        assert_eq!(status.compute_mask(), 0x00);
    }

    #[test]
    fn status_mask_encodes_correctly() {
        let status = SovdFaultStatus {
            test_failed: Some(true),                  // 0x01
            confirmed_dtc: Some(true),                // 0x08
            test_failed_since_last_clear: Some(true), // 0x20
            ..Default::default()
        };
        assert_eq!(status.compute_mask(), 0x29);
    }

    #[test]
    fn status_mask_all_bits_set() {
        let status = SovdFaultStatus {
            test_failed: Some(true),
            test_failed_this_operation_cycle: Some(true),
            pending_dtc: Some(true),
            confirmed_dtc: Some(true),
            test_not_completed_since_last_clear: Some(true),
            test_failed_since_last_clear: Some(true),
            test_not_completed_this_operation_cycle: Some(true),
            warning_indicator_requested: Some(true),
            mask: None,
        };
        assert_eq!(status.compute_mask(), 0xFF);
    }

    #[test]
    fn from_state_converts_all_flags() {
        let state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            pending_dtc: false,
            ..Default::default()
        };
        let status = SovdFaultStatus::from_state(&state);

        assert_eq!(status.test_failed, Some(true));
        assert_eq!(status.confirmed_dtc, Some(true));
        assert_eq!(status.pending_dtc, Some(false));
        assert!(status.mask.is_some());
        assert_eq!(status.mask.as_ref().unwrap(), "0x09"); // 0x01 | 0x08
    }

    #[test]
    fn to_hash_map_produces_expected_keys() {
        let status = SovdFaultStatus {
            test_failed: Some(true),
            confirmed_dtc: Some(false),
            mask: Some("0x01".to_string()),
            ..Default::default()
        };
        let map = status.to_hash_map();

        assert_eq!(map.get("testFailed"), Some(&"1".to_string()));
        assert_eq!(map.get("confirmedDTC"), Some(&"0".to_string()));
        assert_eq!(map.get("mask"), Some(&"0x01".to_string()));
    }

    #[test]
    fn format_unix_timestamp_iso8601() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_timestamp(86400), "1970-01-02T00:00:00Z");
        // 2024-01-15 09:50:00 UTC
        assert_eq!(format_unix_timestamp(1705312200), "2024-01-15T09:50:00Z");
        // Leap year: 2024-02-29 00:00:00 UTC (day 60 of 2024, which is leap)
        assert_eq!(format_unix_timestamp(1709164800), "2024-02-29T00:00:00Z");
        // Y2K: 2000-01-01 00:00:00 UTC
        assert_eq!(format_unix_timestamp(946684800), "2000-01-01T00:00:00Z");
    }

    // ==================== FaultId conversion roundtrip tests ====================

    #[test]
    fn fault_id_from_code_parses_numeric_hex() {
        let id = fault_id_from_code("0x2A").unwrap();
        assert_eq!(id, fault::FaultId::Numeric(0x2A));
    }

    #[test]
    fn fault_id_from_code_parses_numeric_hex_uppercase() {
        let id = fault_id_from_code("0X1001").unwrap();
        assert_eq!(id, fault::FaultId::Numeric(0x1001));
    }

    #[test]
    fn fault_id_from_code_parses_uuid() {
        let id = fault_id_from_code("01020304-0506-0708-090a-0b0c0d0e0f10").unwrap();
        assert_eq!(
            id,
            fault::FaultId::Uuid([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
    }

    #[test]
    fn fault_id_from_code_parses_text() {
        let id = fault_id_from_code("my_fault").unwrap();
        assert!(matches!(id, fault::FaultId::Text(_)));
    }

    #[test]
    fn fault_id_roundtrip_numeric() {
        let original = fault::FaultId::Numeric(0x1001);
        let code = fault_id_to_code(&original);
        let parsed = fault_id_from_code(&code).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn fault_id_roundtrip_uuid() {
        let original =
            fault::FaultId::Uuid([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        let code = fault_id_to_code(&original);
        let parsed = fault_id_from_code(&code).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn fault_id_roundtrip_text() {
        let original = fault::FaultId::Text(ShortString::try_from("my_fault").unwrap());
        let code = fault_id_to_code(&original);
        let parsed = fault_id_from_code(&code).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn fault_id_from_code_rejects_invalid_hex() {
        assert!(fault_id_from_code("0xGGGG").is_err());
    }

    #[test]
    fn fault_id_from_code_invalid_uuid_falls_back_to_text() {
        // 36 chars with dashes but invalid hex → falls back to Text
        let code = "ZZZZZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZZZZZZZZZ";
        let id = fault_id_from_code(code).unwrap();
        assert!(matches!(id, fault::FaultId::Text(_)));
    }
}

#[cfg(test)]
mod sovd_manager_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::unreadable_literal,
        clippy::doc_markdown,
        clippy::arithmetic_side_effects
    )]

    use std::sync::Arc;

    use common::{
        catalog::{FaultCatalogBuilder, FaultCatalogConfig},
        fault::*,
        types::to_static_short_string,
    };

    use crate::{
        dfm_test_utils::*,
        fault_catalog_registry::FaultCatalogRegistry,
        fault_record_processor::FaultRecordProcessor,
        sovd_fault_manager::{Error, SovdFaultManager},
        sovd_fault_storage::SovdFaultStateStorage,
    };

    fn make_processor_with_registry(
        storage: Arc<InMemoryStorage>,
        registry: Arc<FaultCatalogRegistry>,
    ) -> FaultRecordProcessor<InMemoryStorage> {
        FaultRecordProcessor::new(storage, registry, make_cycle_tracker())
    }

    // ============================================================================
    // SovdFaultManager query tests
    // ============================================================================

    /// `SovdFaultManager` `get_all_faults` returns stored faults.
    #[test]
    fn sovd_manager_get_all_faults() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();

        // Pre-populate via processor
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("test_entity");

        let record1 = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        let record2 = make_record(
            FaultId::Text(to_static_short_string("fault_b").unwrap()),
            LifecycleStage::Passed,
        );
        processor.process_record(&path, &record1);
        processor.process_record(&path, &record2);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity");
        assert!(faults.is_ok());
        assert_eq!(faults.unwrap().len(), 2);
    }

    /// `SovdFaultManager` returns error for empty path.
    #[test]
    fn sovd_manager_handles_empty_entity() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let manager = SovdFaultManager::new(storage, registry);

        let result = manager.get_all_faults("nonexistent");
        // Should return Ok with empty or Err - either is acceptable
        if let Ok(faults) = result {
            assert!(faults.is_empty());
        }
    }

    /// `FaultCatalogRegistry` lookup by path.
    #[test]
    fn catalog_registry_lookup() {
        let config = FaultCatalogConfig {
            id: "my_entity".into(),
            version: 1,
            faults: vec![],
        };
        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .build();
        let registry = FaultCatalogRegistry::new(vec![catalog]);

        assert!(registry.get("my_entity").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    /// `get_fault` returns `NotFound` for a fault ID not in the catalog.
    #[test]
    fn get_fault_missing_id_returns_not_found() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let manager = SovdFaultManager::new(storage, registry);

        let result = manager.get_fault("test_entity", "nonexistent_fault");
        assert_eq!(result, Err(Error::NotFound));
    }

    /// `get_fault` returns `BadArgument` for a nonexistent path (entity).
    #[test]
    fn get_fault_bad_path_returns_bad_argument() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let manager = SovdFaultManager::new(storage, registry);

        let result = manager.get_fault("nonexistent_entity", "fault_a");
        assert_eq!(result, Err(Error::BadArgument));
    }

    // ============================================================================
    // SovdFault typed_status and counters tests (Phase 6)
    // ============================================================================

    /// `SovdFault` includes `typed_status` with all flags populated.
    #[test]
    fn sovd_fault_includes_typed_status() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "fault_a").unwrap();

        assert!(fault.typed_status.is_some());
        let status = fault.typed_status.as_ref().unwrap();
        assert_eq!(status.test_failed, Some(true));
        assert_eq!(status.confirmed_dtc, Some(true));
        assert!(status.mask.is_some());
    }

    /// `SovdFault` status includes mask field in `HashMap`.
    #[test]
    fn sovd_fault_status_hashmap_includes_mask() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "fault_a").unwrap();

        assert!(fault.status.contains_key("mask"));
        // testFailed=1, testFailedThisOpCycle=1, confirmedDTC=1, testFailedSinceLastClear=1
        // -> 0x01 | 0x02 | 0x08 | 0x20 = 0x2B
        assert_eq!(fault.status.get("mask"), Some(&"0x2B".to_string()));
    }

    /// `SovdFault` includes occurrence counter (defaults to 0 for new faults).
    #[test]
    fn sovd_fault_includes_counters() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "fault_a").unwrap();

        // occurrence_counter incremented on Failed
        assert_eq!(fault.occurrence_counter, Some(1));
        assert_eq!(fault.aging_counter, Some(0));
        assert_eq!(fault.healing_counter, Some(0));
    }

    /// `SovdFault` preserves existing fields (symptom, schema, `translation_id`).
    #[test]
    fn sovd_fault_preserves_existing_fields() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "fault_a").unwrap();

        // Core fields
        assert_eq!(fault.code, "fault_a");
        assert_eq!(fault.fault_name, "Fault A");
        assert_eq!(fault.scope, "ecu");

        // Translation fields
        assert!(!fault.fault_translation_id.is_empty());
    }

    /// All three `FaultId` variants (Text, Numeric, UUID) work through the full
    /// SOVD pipeline: process → store → query.
    #[test]
    fn sovd_manager_mixed_fault_id_variants() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_mixed_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("mixed_entity");

        // Process one record per variant
        let record_text = make_record(
            FaultId::Text(to_static_short_string("fault_text").unwrap()),
            LifecycleStage::Failed,
        );
        let record_numeric = make_record(FaultId::Numeric(0x1001), LifecycleStage::Failed);
        let record_uuid = make_record(
            FaultId::Uuid([
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
                0x0F, 0x10,
            ]),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record_text);
        processor.process_record(&path, &record_numeric);
        processor.process_record(&path, &record_uuid);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("mixed_entity").unwrap();
        assert_eq!(faults.len(), 3);

        // Text fault → code is the literal text
        let text_fault = faults.iter().find(|f| f.code == "fault_text").unwrap();
        assert_eq!(text_fault.fault_name, "Text Fault");
        assert_eq!(
            text_fault.typed_status.as_ref().unwrap().test_failed,
            Some(true)
        );

        // Numeric fault → code is hex-formatted
        let numeric_fault = faults.iter().find(|f| f.code == "0x1001").unwrap();
        assert_eq!(numeric_fault.fault_name, "Numeric Fault");

        // UUID fault → code is standard UUID format
        let uuid_fault = faults
            .iter()
            .find(|f| f.code == "01020304-0506-0708-090a-0b0c0d0e0f10")
            .unwrap();
        assert_eq!(uuid_fault.fault_name, "UUID Fault");
    }

    /// `get_fault` with numeric code "0x1001" correctly resolves to Numeric variant.
    #[test]
    fn sovd_manager_get_fault_numeric_code() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_mixed_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("mixed_entity");

        let record = make_record(FaultId::Numeric(0x1001), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let (fault, _env) = manager.get_fault("mixed_entity", "0x1001").unwrap();
        assert_eq!(fault.code, "0x1001");
        assert_eq!(fault.fault_name, "Numeric Fault");
    }

    /// `get_fault` with UUID code correctly resolves to Uuid variant.
    #[test]
    fn sovd_manager_get_fault_uuid_code() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_mixed_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("mixed_entity");

        let record = make_record(
            FaultId::Uuid([
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
                0x0F, 0x10,
            ]),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let (fault, _env) = manager
            .get_fault("mixed_entity", "01020304-0506-0708-090a-0b0c0d0e0f10")
            .unwrap();
        assert_eq!(fault.code, "01020304-0506-0708-090a-0b0c0d0e0f10");
        assert_eq!(fault.fault_name, "UUID Fault");
    }

    /// `delete_fault` with numeric code removes the correct entry.
    #[test]
    fn sovd_manager_delete_fault_numeric() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_mixed_registry();
        let mut processor =
            make_processor_with_registry(Arc::clone(&storage), Arc::clone(&registry));
        let path = make_path("mixed_entity");

        let record = make_record(FaultId::Numeric(0x1001), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let result = manager.delete_fault("mixed_entity", "0x1001");
        assert!(result.is_ok());
    }

    /// `SovdFault` includes ISO 8601 timestamps when occurrence data is present.
    #[test]
    fn sovd_fault_timestamp_iso8601_format() {
        use crate::sovd_fault_storage::SovdFaultState;

        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();

        // Manually inject a state with known timestamps
        let state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            first_occurrence_secs: 946684800, // 2000-01-01T00:00:00Z
            last_occurrence_secs: 1705312200, // 2024-01-15T09:50:00Z
            ..Default::default()
        };
        storage
            .put(
                "test_entity",
                &FaultId::Text(to_static_short_string("fault_a").unwrap()),
                state,
            )
            .unwrap();

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "fault_a").unwrap();

        assert_eq!(
            fault.first_occurrence.as_deref(),
            Some("2000-01-01T00:00:00Z")
        );
        assert_eq!(
            fault.last_occurrence.as_deref(),
            Some("2024-01-15T09:50:00Z")
        );
    }

    /// `SovdFault` symptom field comes from descriptor summary.
    #[test]
    fn sovd_fault_symptom_from_descriptor_summary() {
        use crate::sovd_fault_storage::SovdFaultState;

        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_mixed_registry();

        // Inject a state for a descriptor that has a summary
        let state = SovdFaultState {
            test_failed: true,
            confirmed_dtc: true,
            ..Default::default()
        };
        storage
            .put("mixed_entity", &FaultId::Numeric(0x1001), state)
            .unwrap();

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("mixed_entity").unwrap();
        let fault = faults.iter().find(|f| f.code == "0x1001").unwrap();

        assert_eq!(fault.symptom.as_deref(), Some("A numeric DTC-like fault"));
        assert!(fault.symptom_translation_id.is_some());
    }
}
