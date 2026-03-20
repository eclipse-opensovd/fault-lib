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

//! Conversions between `SovdFault` (heap-allocated) and `IpcSovdFault`
//! (fixed-size, IPC-safe).
//!
//! # Lossy conversion policy
//!
//! `SovdFault` -> `IpcSovdFault`:
//! - `String` fields silently truncated via `from_str_truncated()`
//! - `HashMap status` / `Option<SovdFaultStatus> typed_status` -> individual bool fields
//! - `Option<String> schema` -> omitted
//! - `Option<bool>` -> `bool` (None -> false)
//! - Timestamps: ISO 8601 `String` -> `u64` seconds (parsed back from our own format)
//!
//! `IpcSovdFault` -> `SovdFault`:
//! - Fixed-size strings -> heap-allocated `String`
//! - Bool fields -> reconstructed `SovdFaultStatus` + `HashMap`
//! - `u64` timestamps -> ISO 8601 strings (using existing `format_unix_timestamp`)

use common::{
    query_protocol::{IpcEnvData, IpcSovdFault},
    types::{LongString, ShortString},
};

use crate::sovd_fault_manager::{SovdEnvData, SovdFault, SovdFaultStatus};

/// Truncating conversion from `&str` to `ShortString`, falling back to empty
/// on encoding errors (should not happen for valid UTF-8).
fn short_str(s: &str) -> ShortString {
    let result = ShortString::from_str_truncated(s).unwrap_or_default();
    if s.len() > result.len() {
        tracing::warn!(
            "IPC ShortString truncation: input {} bytes -> {} bytes",
            s.len(),
            result.len()
        );
    }
    result
}

/// Truncating conversion from `&str` to `LongString`.
fn long_str(s: &str) -> LongString {
    let result = LongString::from_str_truncated(s).unwrap_or_default();
    if s.len() > result.len() {
        tracing::warn!(
            "IPC LongString truncation: input {} bytes -> {} bytes",
            s.len(),
            result.len()
        );
    }
    result
}

/// Convert a [`SovdFault`] to its IPC-safe equivalent.
///
/// This conversion is lossy - see module docs for truncation policy.
pub fn sovd_fault_to_ipc(fault: &SovdFault) -> IpcSovdFault {
    let status = fault.typed_status.as_ref();

    let status_mask = status.map_or(0, super::sovd_fault_manager::SovdFaultStatus::compute_mask);

    IpcSovdFault {
        code: short_str(&fault.code),
        display_code: short_str(&fault.display_code),
        scope: short_str(&fault.scope),
        fault_name: short_str(&fault.fault_name),
        fault_translation_id: short_str(&fault.fault_translation_id),
        severity: fault.severity,

        status_mask,
        test_failed: status.and_then(|s| s.test_failed).unwrap_or(false),
        test_failed_this_operation_cycle: status
            .and_then(|s| s.test_failed_this_operation_cycle)
            .unwrap_or(false),
        pending_dtc: status.and_then(|s| s.pending_dtc).unwrap_or(false),
        confirmed_dtc: status.and_then(|s| s.confirmed_dtc).unwrap_or(false),
        test_not_completed_since_last_clear: status
            .and_then(|s| s.test_not_completed_since_last_clear)
            .unwrap_or(false),
        test_failed_since_last_clear: status
            .and_then(|s| s.test_failed_since_last_clear)
            .unwrap_or(false),
        test_not_completed_this_operation_cycle: status
            .and_then(|s| s.test_not_completed_this_operation_cycle)
            .unwrap_or(false),
        warning_indicator_requested: status
            .and_then(|s| s.warning_indicator_requested)
            .unwrap_or(false),

        occurrence_counter: fault.occurrence_counter.unwrap_or(0),
        aging_counter: fault.aging_counter.unwrap_or(0),
        healing_counter: fault.healing_counter.unwrap_or(0),
        first_occurrence_secs: parse_iso_timestamp(fault.first_occurrence.as_deref()),
        last_occurrence_secs: parse_iso_timestamp(fault.last_occurrence.as_deref()),

        symptom: fault
            .symptom
            .as_ref()
            .map(|s| long_str(s))
            .unwrap_or_default(),
        has_symptom: fault.symptom.is_some(),
        symptom_translation_id: fault
            .symptom_translation_id
            .as_ref()
            .map(|s| short_str(s))
            .unwrap_or_default(),
        has_symptom_translation_id: fault.symptom_translation_id.is_some(),
    }
}

/// Convert an [`IpcSovdFault`] back to a heap-allocated [`SovdFault`].
pub fn ipc_fault_to_sovd(ipc: &IpcSovdFault) -> SovdFault {
    let typed_status = SovdFaultStatus {
        test_failed: Some(ipc.test_failed),
        test_failed_this_operation_cycle: Some(ipc.test_failed_this_operation_cycle),
        pending_dtc: Some(ipc.pending_dtc),
        confirmed_dtc: Some(ipc.confirmed_dtc),
        test_not_completed_since_last_clear: Some(ipc.test_not_completed_since_last_clear),
        test_failed_since_last_clear: Some(ipc.test_failed_since_last_clear),
        test_not_completed_this_operation_cycle: Some(ipc.test_not_completed_this_operation_cycle),
        warning_indicator_requested: Some(ipc.warning_indicator_requested),
        mask: Some(alloc::format!("0x{:02X}", ipc.status_mask)),
    };

    SovdFault {
        code: ipc.code.to_string(),
        display_code: ipc.display_code.to_string(),
        scope: ipc.scope.to_string(),
        fault_name: ipc.fault_name.to_string(),
        fault_translation_id: ipc.fault_translation_id.to_string(),
        severity: ipc.severity,
        status: typed_status.to_hash_map(),
        typed_status: Some(typed_status),
        symptom: if ipc.has_symptom {
            Some(ipc.symptom.to_string())
        } else {
            None
        },
        symptom_translation_id: if ipc.has_symptom_translation_id {
            Some(ipc.symptom_translation_id.to_string())
        } else {
            None
        },
        schema: None,
        occurrence_counter: Some(ipc.occurrence_counter),
        aging_counter: Some(ipc.aging_counter),
        healing_counter: Some(ipc.healing_counter),
        first_occurrence: if ipc.first_occurrence_secs > 0 {
            Some(crate::sovd_fault_manager::format_unix_timestamp(
                ipc.first_occurrence_secs,
            ))
        } else {
            None
        },
        last_occurrence: if ipc.last_occurrence_secs > 0 {
            Some(crate::sovd_fault_manager::format_unix_timestamp(
                ipc.last_occurrence_secs,
            ))
        } else {
            None
        },
    }
}

/// Convert a [`SovdEnvData`] (`HashMap<String, String>`) to [`IpcEnvData`].
///
/// Truncates keys/values to `ShortString` capacity. Entries beyond capacity
/// (8) are silently dropped.
pub fn env_data_to_ipc(env: &SovdEnvData) -> IpcEnvData {
    use iceoryx2_bb_container::vector::Vector;
    let mut ipc = IpcEnvData::new();
    for (k, v) in env {
        if ipc.is_full() {
            break;
        }
        let _ = ipc.push((short_str(k), short_str(v)));
    }
    ipc
}

/// Convert [`IpcEnvData`] back to [`SovdEnvData`].
pub fn ipc_env_data_to_sovd(ipc: &IpcEnvData) -> SovdEnvData {
    ipc.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Parse an ISO 8601 timestamp string (our format: "YYYY-MM-DDThh:mm:ssZ")
/// back to Unix seconds. Returns 0 if None or unparseable.
///
/// This is the inverse of `sovd_fault_manager::format_unix_timestamp`.
/// Only supports the exact format we produce - no timezone offsets.
/// Dates before 1970-01-01 return 0 (pre-epoch).
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]
fn parse_iso_timestamp(s: Option<&str>) -> u64 {
    let Some(s) = s else { return 0 };
    // Expected: "2024-01-15T09:50:00Z" (exactly 20 chars)
    if s.len() != 20 || !s.ends_with('Z') {
        return 0;
    }
    let b = s.as_bytes();
    let (Some(year_b), Some(month_b), Some(day_b), Some(hour_b), Some(min_b), Some(sec_b)) = (
        b.get(0..4),
        b.get(5..7),
        b.get(8..10),
        b.get(11..13),
        b.get(14..16),
        b.get(17..19),
    ) else {
        return 0;
    };
    let year = parse_u64(year_b);
    let month = parse_u64(month_b);
    let day = parse_u64(day_b);
    let hour = parse_u64(hour_b);
    let min = parse_u64(min_b);
    let sec = parse_u64(sec_b);

    if year < 1970 || month == 0 || day == 0 {
        return 0;
    }
    if month > 12 || day > 31 {
        return 0;
    }
    if hour >= 24 || min >= 60 || sec >= 60 {
        return 0;
    }

    days_from_civil(year as i64, month as u32, day as u32) * 86_400 + hour * 3_600 + min * 60 + sec
}

/// Parse ASCII decimal digits to u64. Returns 0 on any non-digit.
#[allow(clippy::arithmetic_side_effects)]
fn parse_u64(bytes: &[u8]) -> u64 {
    let mut result = 0u64;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return 0;
        }
        result = result * 10 + u64::from(b - b'0');
    }
    result
}

/// Convert (year, month, day) to days since Unix epoch (1970-01-01).
/// Algorithm: Howard Hinnant's `days_from_civil`.
///
/// Returns 0 for dates before 1970-01-01 (negative day counts).
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names
)]
fn days_from_civil(y: i64, m: u32, d: u32) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + i64::from(doe) - 719_468;
    if days < 0 {
        return 0;
    }
    days as u64
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
    use std::sync::Arc;

    use common::{
        fault::{FaultId, LifecycleStage},
        types::to_static_short_string,
    };
    use iceoryx2_bb_container::string::String as IceString;

    use super::*;
    use crate::{
        dfm_test_utils::*, fault_record_processor::FaultRecordProcessor,
        sovd_fault_manager::SovdFaultManager,
    };

    /// Helper: create a `SovdFault` via the real pipeline.
    fn make_real_fault() -> SovdFault {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            make_cycle_tracker(),
        );
        let path = make_path("test_entity");
        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let manager = SovdFaultManager::new(storage, registry);
        let faults = manager.get_all_faults("test_entity").unwrap();
        faults.into_iter().find(|f| f.code == "fault_a").unwrap()
    }

    #[test]
    fn roundtrip_preserves_core_fields() {
        let original = make_real_fault();
        let ipc = sovd_fault_to_ipc(&original);
        let restored = ipc_fault_to_sovd(&ipc);

        assert_eq!(restored.code, original.code);
        assert_eq!(restored.display_code, original.display_code);
        assert_eq!(restored.scope, original.scope);
        assert_eq!(restored.fault_name, original.fault_name);
        assert_eq!(restored.severity, original.severity);
    }

    #[test]
    fn roundtrip_preserves_status_flags() {
        let original = make_real_fault();
        let ipc = sovd_fault_to_ipc(&original);
        let restored = ipc_fault_to_sovd(&ipc);

        let orig_status = original.typed_status.as_ref().unwrap();
        let rest_status = restored.typed_status.as_ref().unwrap();

        assert_eq!(rest_status.test_failed, orig_status.test_failed);
        assert_eq!(rest_status.confirmed_dtc, orig_status.confirmed_dtc);
        assert_eq!(rest_status.pending_dtc, orig_status.pending_dtc);
        assert_eq!(
            rest_status.warning_indicator_requested,
            orig_status.warning_indicator_requested
        );
    }

    #[test]
    fn roundtrip_preserves_counters() {
        let original = make_real_fault();
        let ipc = sovd_fault_to_ipc(&original);
        let restored = ipc_fault_to_sovd(&ipc);

        assert_eq!(restored.occurrence_counter, original.occurrence_counter);
        assert_eq!(restored.aging_counter, original.aging_counter);
        assert_eq!(restored.healing_counter, original.healing_counter);
    }

    #[test]
    fn roundtrip_preserves_symptom() {
        let original = make_real_fault();
        let ipc = sovd_fault_to_ipc(&original);
        let restored = ipc_fault_to_sovd(&ipc);

        // fault_a in make_text_registry has no summary -> symptom is None
        assert_eq!(restored.symptom, original.symptom);
        assert_eq!(
            restored.symptom_translation_id,
            original.symptom_translation_id
        );
    }

    #[test]
    fn known_lossy_fields_are_expected() {
        let original = make_real_fault();
        let ipc = sovd_fault_to_ipc(&original);
        let restored = ipc_fault_to_sovd(&ipc);

        // schema is always None after IPC roundtrip
        assert!(restored.schema.is_none());
        // status HashMap is reconstructed (may differ in iteration order)
        assert!(!restored.status.is_empty());
    }

    #[test]
    fn truncation_does_not_error() {
        // Create a fault with a very long name
        let fault = SovdFault {
            code: "x".repeat(200),
            fault_name: "y".repeat(200),
            symptom: Some("z".repeat(300)),
            ..SovdFault::default()
        };

        let ipc = sovd_fault_to_ipc(&fault);
        // Should truncate, not panic or error
        assert!(ipc.code.as_bytes().len() <= 64);
        assert!(ipc.fault_name.as_bytes().len() <= 64);
        assert!(ipc.symptom.as_bytes().len() <= 128);
        assert!(ipc.has_symptom);
    }

    #[test]
    fn env_data_roundtrip() {
        let mut env = SovdEnvData::new();
        env.insert("temp".into(), "42".into());
        env.insert("pressure".into(), "1013".into());

        let ipc = env_data_to_ipc(&env);
        let restored = ipc_env_data_to_sovd(&ipc);

        assert_eq!(restored.get("temp"), Some(&"42".into()));
        assert_eq!(restored.get("pressure"), Some(&"1013".into()));
    }

    #[test]
    fn env_data_overflow_truncates() {
        use iceoryx2_bb_container::vector::Vector;
        let mut env = SovdEnvData::new();
        for i in 0..20 {
            env.insert(alloc::format!("key_{i}"), alloc::format!("val_{i}"));
        }
        let ipc = env_data_to_ipc(&env);
        assert_eq!(ipc.len(), 8); // capacity limit
    }

    #[test]
    fn timestamp_roundtrip() {
        let ts = 1705312200u64; // 2024-01-15T09:50:00Z
        let iso = crate::sovd_fault_manager::format_unix_timestamp(ts);
        let parsed = parse_iso_timestamp(Some(&iso));
        assert_eq!(parsed, ts);
    }

    #[test]
    fn timestamp_none_returns_zero() {
        assert_eq!(parse_iso_timestamp(None), 0);
    }

    #[test]
    fn timestamp_invalid_returns_zero() {
        assert_eq!(parse_iso_timestamp(Some("not-a-date")), 0);
        assert_eq!(parse_iso_timestamp(Some("")), 0);
    }

    #[test]
    fn timestamp_epoch_zero() {
        let iso = crate::sovd_fault_manager::format_unix_timestamp(0);
        assert_eq!(iso, "1970-01-01T00:00:00Z");
        let parsed = parse_iso_timestamp(Some(&iso));
        assert_eq!(parsed, 0);
    }

    #[test]
    fn timestamp_pre_epoch_returns_zero() {
        // Year 1969 is before Unix epoch
        assert_eq!(parse_iso_timestamp(Some("1969-12-31T23:59:59Z")), 0);
    }

    #[test]
    fn timestamp_invalid_month_day_returns_zero() {
        assert_eq!(parse_iso_timestamp(Some("2024-13-01T00:00:00Z")), 0); // month 13
        assert_eq!(parse_iso_timestamp(Some("2024-01-32T00:00:00Z")), 0); // day 32
    }

    #[test]
    fn format_timestamp_u64_max_clamps_to_year_9999() {
        let result = crate::sovd_fault_manager::format_unix_timestamp(u64::MAX);
        assert_eq!(result, "9999-12-31T23:59:59Z");
    }

    #[test]
    fn format_timestamp_year_9999_boundary() {
        // Exact boundary: 253_402_300_799 = 9999-12-31T23:59:59Z
        let result = crate::sovd_fault_manager::format_unix_timestamp(253_402_300_799);
        assert_eq!(result, "9999-12-31T23:59:59Z");

        // One second over the boundary still clamps
        let result_over = crate::sovd_fault_manager::format_unix_timestamp(253_402_300_800);
        assert_eq!(result_over, "9999-12-31T23:59:59Z");
    }
}
