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

use crate::ResetPolicy;
use crate::debounce::DebounceMode;
use crate::ids::*;
use crate::types::*;
use iceoryx2::prelude::ZeroCopySend;
use iceoryx2_bb_container::vector::StaticVec;
use serde::{Deserialize, Serialize};

pub type ComplianceVec = StaticVec<ComplianceTag, 8>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum FaultId {
    Numeric(u32),      // e.g., DTC-like
    Text(ShortString), // human-stable symbolic ID
    Uuid([u8; 16]),    // global uniqueness if needed
}

/// Canonical fault type buckets used for analytics and tooling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum FaultType {
    Hardware,
    Software,
    Communication,
    Configuration,
    Timing,
    Power,
    /// Escape hatch for domain-specific groupings until the enum grows.
    Custom(ShortString),
}

/// Align severities to DLT-like levels, stable for logging & UI filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum FaultSeverity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// Compliance/regulatory tags drive escalation, retention, and workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum ComplianceTag {
    EmissionRelevant,
    SafetyCritical,
    SecurityRelevant,
    LegalHold,
}

/// Lifecycle phase of the reporting component/system (for policy gating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum LifecyclePhase {
    Init,
    Running,
    Suspend,
    Resume,
    Shutdown,
}

/// State of a fault’s lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum LifecycleStage {
    NotTested, // test not executed yet for this reporting window
    PreFailed, // initial failure observed but still within debounce/pending window
    Failed,    // confirmed failure (debounce satisfied / threshold met)
    PrePassed, // transitioning back to healthy; stability window accumulating
    Passed,    // test executed and passed (healthy condition)
}

/// Immutable, compile-time describer of a fault type (identity + defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultDescriptor {
    pub id: FaultId,

    pub name: ShortString,
    pub summary: Option<LongString>,

    pub category: FaultType,
    pub severity: FaultSeverity,
    pub compliance: ComplianceVec,

    pub reporter_side_debounce: Option<DebounceMode>,
    pub reporter_side_reset: Option<ResetPolicy>,
    pub manager_side_debounce: Option<DebounceMode>,
    pub manager_side_reset: Option<ResetPolicy>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, ZeroCopySend)]
#[repr(C)]
pub struct IpcTimestamp {
    pub seconds_since_epoch: u64,
    pub nanoseconds: u32,
}

/// Concrete record produced on each report() call, also logged.
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub struct FaultRecord {
    pub id: FaultId,
    pub time: IpcTimestamp,
    pub source: SourceId,
    pub lifecycle_phase: LifecyclePhase,
    pub lifecycle_stage: LifecycleStage,
    pub env_data: MetadataVec,
}
