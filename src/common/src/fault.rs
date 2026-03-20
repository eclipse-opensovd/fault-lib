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

use iceoryx2::prelude::ZeroCopySend;
use iceoryx2_bb_container::vector::StaticVec;
use serde::{Deserialize, Serialize};

use crate::{
    ResetPolicy,
    debounce::DebounceMode,
    ids::SourceId,
    types::{LongString, MetadataVec, ShortString},
};

/// Fixed-capacity vector of compliance tags (max 8).
pub type ComplianceVec = StaticVec<ComplianceTag, 8>;

/// Unique identifier for a fault.
///
/// Three representations are supported to cover different use cases:
/// numeric codes for DTC-like systems, human-readable text, and UUIDs
/// for globally unique identification.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ZeroCopySend,
)]
#[repr(C)]
pub enum FaultId {
    /// Numeric identifier (e.g. DTC-like `0x7001`).
    Numeric(u32),
    /// Human-readable symbolic identifier (e.g. `"hvac.blower.mismatch"`).
    Text(ShortString),
    /// 128-bit UUID for globally unique identification.
    Uuid([u8; 16]),
}

/// Canonical fault type buckets used for analytics and tooling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum FaultType {
    /// Hardware fault (sensor, actuator, etc.).
    Hardware,
    /// Software fault (assertion, logic error, etc.).
    Software,
    /// Communication fault (bus timeout, CRC mismatch, etc.).
    Communication,
    /// Configuration fault (invalid parameter, schema mismatch, etc.).
    Configuration,
    /// Timing fault (deadline miss, watchdog, etc.).
    Timing,
    /// Power-related fault (undervoltage, brownout, etc.).
    Power,
    /// Escape hatch for domain-specific groupings until the enum grows.
    Custom(ShortString),
}

/// Align severities to DLT-like levels, stable for logging & UI filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum FaultSeverity {
    /// Finest-grained diagnostic output.
    Trace,
    /// Diagnostic output useful during development.
    Debug,
    /// Informational event (no error).
    Info,
    /// Non-critical issue that may require attention.
    Warn,
    /// Significant error requiring action.
    Error,
    /// Unrecoverable failure.
    Fatal,
}

/// Compliance/regulatory tags drive escalation, retention, and workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum ComplianceTag {
    /// Fault is relevant for emissions regulation (e.g. OBD-II).
    EmissionRelevant,
    /// Fault relates to a safety-critical function (e.g. ISO 26262).
    SafetyCritical,
    /// Fault has security implications.
    SecurityRelevant,
    /// Fault data must be retained for legal/regulatory purposes.
    LegalHold,
}

/// Lifecycle phase of the reporting component/system (for policy gating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum LifecyclePhase {
    /// System initialising.
    Init,
    /// Normal operation.
    Running,
    /// Entering low-power / sleep state.
    Suspend,
    /// Waking from suspend.
    Resume,
    /// Orderly shutdown in progress.
    Shutdown,
}

/// State of a fault’s lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum LifecycleStage {
    /// Test not executed yet for this reporting window.
    NotTested,
    /// Initial failure observed but still within debounce/pending window.
    PreFailed,
    /// Confirmed failure (debounce satisfied / threshold met).
    Failed,
    /// Transitioning back to healthy; stability window accumulating.
    PrePassed,
    /// Test executed and passed (healthy condition).
    Passed,
}

impl LifecycleStage {
    /// Check whether a transition from `self` to `to` is valid per the
    /// ISO 14229 / DFM fault lifecycle state machine.
    ///
    /// Valid transitions:
    /// ```text
    /// NotTested → PreFailed | PrePassed | Failed | Passed
    /// PreFailed → Failed | PrePassed | NotTested
    /// Failed    → PrePassed | Passed | NotTested
    /// PrePassed → Passed | PreFailed | NotTested
    /// Passed    → PreFailed | Failed | NotTested
    /// ```
    ///
    /// Self-transitions (e.g. Failed → Failed) are allowed as no-ops.
    #[must_use]
    pub fn is_valid_transition(&self, to: &LifecycleStage) -> bool {
        if self == to {
            return true;
        }
        matches!(
            (self, to),
            (
                LifecycleStage::NotTested,
                LifecycleStage::PreFailed
                    | LifecycleStage::PrePassed
                    | LifecycleStage::Failed
                    | LifecycleStage::Passed
            ) | (
                LifecycleStage::PreFailed,
                LifecycleStage::Failed | LifecycleStage::PrePassed | LifecycleStage::NotTested
            ) | (
                LifecycleStage::Failed,
                LifecycleStage::PrePassed | LifecycleStage::Passed | LifecycleStage::NotTested
            ) | (
                LifecycleStage::PrePassed,
                LifecycleStage::Passed | LifecycleStage::PreFailed | LifecycleStage::NotTested
            ) | (
                LifecycleStage::Passed,
                LifecycleStage::PreFailed | LifecycleStage::Failed | LifecycleStage::NotTested
            )
        )
    }
}

/// Immutable, compile-time describer of a fault type (identity + defaults).
///
/// # Debounce/Reset Fields
///
/// - `reporter_side_debounce`: Applied before IPC send, reduces network traffic
/// - `manager_side_debounce`: Applied at DFM, enables multi-source aggregation
///   (**reserved for future implementation**)
/// - `reporter_side_reset`: Clears fault after condition passes for duration
///   (**reserved for future implementation**)
/// - `manager_side_reset`: Clears fault after global aging policy
///   (**reserved for future implementation**)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultDescriptor {
    /// Unique fault identifier.
    pub id: FaultId,

    /// Short display name for the fault.
    pub name: ShortString,
    /// Human-readable summary (used as SOVD symptom field).
    pub summary: Option<LongString>,

    /// Fault category bucket (hardware, software, comms, etc.).
    pub category: FaultType,
    /// Default severity level.
    pub severity: FaultSeverity,
    /// Regulatory/compliance tags.
    pub compliance: ComplianceVec,

    /// Reporter-side debounce configuration.
    /// Applied in `Reporter::publish()` to filter events before IPC transport.
    pub reporter_side_debounce: Option<DebounceMode>,
    /// Reporter-side reset/aging configuration.
    /// **Status: Reserved for future implementation.**
    pub reporter_side_reset: Option<ResetPolicy>,
    /// Manager-side debounce configuration.
    /// **Status: Reserved for future implementation.**
    pub manager_side_debounce: Option<DebounceMode>,
    /// Manager-side reset/aging configuration.
    /// When set, `confirmed_dtc` stays latched after the fault passes until
    /// the aging policy trigger (power cycles, operation cycles, or time) is met.
    pub manager_side_reset: Option<ResetPolicy>,
}

/// IPC-safe timestamp with epoch-relative seconds and nanoseconds.
///
/// `std::time::SystemTime` has no stable memory layout, so this type
/// is used for all timestamps that cross IPC boundaries.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, ZeroCopySend)]
#[repr(C)]
pub struct IpcTimestamp {
    /// Seconds elapsed since the Unix epoch.
    pub seconds_since_epoch: u64,
    /// Sub-second nanoseconds component (0…999 999 999).
    pub nanoseconds: u32,
}

impl IpcTimestamp {
    /// Maximum valid value for the `nanoseconds` field.
    pub const MAX_NANOS: u32 = 999_999_999;

    /// Create a new `IpcTimestamp` with validation.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `nanoseconds` exceeds 999,999,999.
    pub fn new(seconds_since_epoch: u64, nanoseconds: u32) -> Result<Self, IpcTimestampError> {
        if nanoseconds > Self::MAX_NANOS {
            return Err(IpcTimestampError::NanosecondsOutOfRange(nanoseconds));
        }
        Ok(Self {
            seconds_since_epoch,
            nanoseconds,
        })
    }
}

/// Error returned when constructing an [`IpcTimestamp`] with invalid fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IpcTimestampError {
    /// Nanoseconds value exceeded the valid range.
    #[error("nanoseconds out of range: {0} (max: 999_999_999)")]
    NanosecondsOutOfRange(u32),
}

/// Concrete record produced on each `report()` call, also logged.
#[derive(Debug, Clone, PartialEq, ZeroCopySend)]
#[repr(C)]
pub struct FaultRecord {
    /// Fault identifier linking this record to its descriptor.
    pub id: FaultId,
    /// Timestamp of the report.
    pub time: IpcTimestamp,
    /// Identity of the reporting component.
    pub source: SourceId,
    /// Current lifecycle phase of the reporter.
    pub lifecycle_phase: LifecyclePhase,
    /// Current lifecycle stage of this fault occurrence.
    pub lifecycle_stage: LifecycleStage,
    /// Free-form key-value environment data.
    pub env_data: MetadataVec,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // ========== FaultId Tests ==========

    #[test]
    fn fault_id_numeric_construction() {
        let id = FaultId::Numeric(42);
        if let FaultId::Numeric(n) = id {
            assert_eq!(n, 42);
        } else {
            panic!("Expected Numeric variant");
        }
    }

    #[test]
    fn fault_id_text_construction() {
        let id = FaultId::Text(ShortString::try_from("test_fault").unwrap());
        if let FaultId::Text(ref s) = id {
            assert_eq!(s.to_string(), "test_fault");
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn fault_id_uuid_construction() {
        let uuid = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let id = FaultId::Uuid(uuid);
        if let FaultId::Uuid(u) = id {
            assert_eq!(u, uuid);
        } else {
            panic!("Expected Uuid variant");
        }
    }

    #[test]
    fn fault_id_equality() {
        let a = FaultId::Numeric(1);
        let b = FaultId::Numeric(1);
        let c = FaultId::Numeric(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn fault_id_ordering() {
        let a = FaultId::Numeric(1);
        let b = FaultId::Numeric(2);
        assert!(a < b);
    }

    #[test]
    fn fault_id_hash_works() {
        let mut set = HashSet::new();
        set.insert(FaultId::Numeric(1));
        set.insert(FaultId::Numeric(2));
        set.insert(FaultId::Numeric(1)); // Duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn fault_id_clone() {
        let a = FaultId::Numeric(42);
        let b = a.clone();
        assert_eq!(a, b);
    }

    // ========== FaultSeverity Tests ==========

    #[test]
    fn fault_severity_copy() {
        let a = FaultSeverity::Error;
        let b = a; // Copy
        assert_eq!(a, b);
    }

    #[test]
    fn fault_severity_all_variants() {
        let severities = [
            FaultSeverity::Trace,
            FaultSeverity::Debug,
            FaultSeverity::Info,
            FaultSeverity::Warn,
            FaultSeverity::Error,
            FaultSeverity::Fatal,
        ];
        assert_eq!(severities.len(), 6);
    }

    // ========== LifecycleStage Tests ==========

    #[test]
    fn lifecycle_stage_copy() {
        let a = LifecycleStage::Failed;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn lifecycle_stage_all_variants() {
        let stages = [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
            LifecycleStage::PrePassed,
            LifecycleStage::Passed,
        ];
        assert_eq!(stages.len(), 5);
    }

    // ========== LifecycleStage::is_valid_transition Tests ==========

    #[test]
    fn lifecycle_self_transitions_are_valid() {
        for stage in [
            LifecycleStage::NotTested,
            LifecycleStage::PreFailed,
            LifecycleStage::Failed,
            LifecycleStage::PrePassed,
            LifecycleStage::Passed,
        ] {
            assert!(
                stage.is_valid_transition(&stage),
                "{stage:?} → {stage:?} should be valid"
            );
        }
    }

    #[test]
    fn lifecycle_valid_transitions() {
        use LifecycleStage::*;
        let valid = [
            (NotTested, PreFailed),
            (NotTested, PrePassed),
            (NotTested, Failed),
            (NotTested, Passed),
            (PreFailed, Failed),
            (PreFailed, PrePassed),
            (PreFailed, NotTested),
            (Failed, PrePassed),
            (Failed, Passed),
            (Failed, NotTested),
            (PrePassed, Passed),
            (PrePassed, PreFailed),
            (PrePassed, NotTested),
            (Passed, PreFailed),
            (Passed, Failed),
            (Passed, NotTested),
        ];
        for (from, to) in valid {
            assert!(
                from.is_valid_transition(&to),
                "{from:?} → {to:?} should be valid"
            );
        }
    }

    #[test]
    fn lifecycle_invalid_transitions() {
        use LifecycleStage::*;
        let invalid = [
            (PreFailed, Passed),
            (Failed, PreFailed),
            (PrePassed, Failed),
            (Passed, PrePassed),
        ];
        for (from, to) in invalid {
            assert!(
                !from.is_valid_transition(&to),
                "{from:?} → {to:?} should be invalid"
            );
        }
    }

    // ========== IpcTimestamp Tests ==========

    #[test]
    fn ipc_timestamp_default_is_zero() {
        let ts = IpcTimestamp::default();
        assert_eq!(ts.seconds_since_epoch, 0);
        assert_eq!(ts.nanoseconds, 0);
    }

    #[test]
    fn ipc_timestamp_construction() {
        let ts = IpcTimestamp {
            seconds_since_epoch: 1_705_312_200,
            nanoseconds: 123_456_789,
        };
        assert_eq!(ts.seconds_since_epoch, 1_705_312_200);
        assert_eq!(ts.nanoseconds, 123_456_789);
    }

    // ========== IpcTimestamp::new validation ==========

    #[test]
    fn ipc_timestamp_new_zero_nanos_ok() {
        let ts = IpcTimestamp::new(0, 0);
        assert!(ts.is_ok());
        let ts = ts.unwrap();
        assert_eq!(ts.seconds_since_epoch, 0);
        assert_eq!(ts.nanoseconds, 0);
    }

    #[test]
    fn ipc_timestamp_new_max_nanos_ok() {
        let ts = IpcTimestamp::new(42, 999_999_999);
        assert!(ts.is_ok());
        let ts = ts.unwrap();
        assert_eq!(ts.seconds_since_epoch, 42);
        assert_eq!(ts.nanoseconds, 999_999_999);
    }

    #[test]
    fn ipc_timestamp_new_nanos_overflow_err() {
        let ts = IpcTimestamp::new(0, 1_000_000_000);
        assert!(ts.is_err());
        assert_eq!(
            ts.unwrap_err(),
            super::IpcTimestampError::NanosecondsOutOfRange(1_000_000_000)
        );
    }

    #[test]
    fn ipc_timestamp_new_max_u32_nanos_err() {
        let ts = IpcTimestamp::new(0, u32::MAX);
        assert!(ts.is_err());
    }

    #[test]
    fn ipc_timestamp_error_display() {
        let err = super::IpcTimestampError::NanosecondsOutOfRange(1_000_000_000);
        let msg = format!("{err}");
        assert!(msg.contains("nanoseconds out of range"));
        assert!(msg.contains("1000000000"));
    }

    // ========== ComplianceTag Tests ==========

    #[test]
    fn compliance_tag_all_variants() {
        let tags = [
            ComplianceTag::EmissionRelevant,
            ComplianceTag::SafetyCritical,
            ComplianceTag::SecurityRelevant,
            ComplianceTag::LegalHold,
        ];
        assert_eq!(tags.len(), 4);
    }

    // ========== FaultType Tests ==========

    #[test]
    fn fault_type_custom() {
        let custom = FaultType::Custom(ShortString::try_from("domain_specific").unwrap());
        if let FaultType::Custom(ref s) = custom {
            assert_eq!(s.to_string(), "domain_specific");
        } else {
            panic!("Expected Custom variant");
        }
    }
}
