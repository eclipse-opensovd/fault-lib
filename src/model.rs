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

use crate::FaultId;
// use crate::DebouncePolicy;
use std::{borrow::Cow, time::SystemTime};

// Shared domain types that move between reporters, sinks, and integrators.

/// Align severities to DLT-like levels, stable for logging & UI filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultSeverity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// Canonical fault type buckets used for analytics and tooling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FaultType {
    Hardware,
    Software,
    Communication,
    Configuration,
    Timing,
    Power,
    /// Escape hatch for domain-specific groupings until the enum grows.
    Custom(&'static str),
}

/// Compliance/regulatory tags drive escalation, retention, and workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComplianceTag {
    EmissionRelevant,
    SafetyCritical,
    SecurityRelevant,
    LegalHold,
}

/// Lifecycle phase of the reporting component/system (for policy gating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecyclePhase {
    Init,
    Running,
    Suspend,
    Resume,
    Shutdown,
}

/// Simplified internal test lifecycle aligned with ISO 14229-1 style semantics.
/// DTC lifecycle (confirmation, pending, aging, etc.) is handled centrally by the DFM.
/// The fault-lib only tracks raw test pass/fail progression + pre-states around debounce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultLifecycleStage {
    NotTested,   // test not executed yet for this reporting window
    PreFailed,   // initial failure observed but still within debounce/pending window
    Failed,      // confirmed failure (debounce satisfied / threshold met)
    PrePassed,   // transitioning back to healthy; stability window accumulating
    Passed,      // test executed and passed (healthy condition)
}

/// Minimal, typed environment data; keep serde-agnostic at the API edge.
#[derive(Debug, Clone)]
pub struct KeyValue {
    pub key: &'static str,
    /// Values stay stringly-typed so logging/IPC layers stay decoupled.
    pub value: String,
}

/// Immutable, compile-time describer of a fault type (identity + defaults).
#[derive(Debug, Clone)]
pub struct FaultDescriptor {
    pub id: crate::ids::FaultId,
    pub name: Cow<'static, str>,
    pub fault_type: FaultType,
    pub default_severity: FaultSeverity,
    pub compliance: Cow<'static, [ComplianceTag]>,
    /// Default debounce/reset; can be overridden per-report via ReportOptions.
    pub debounce: Option<crate::config::DebouncePolicy>,
    pub reset: Option<crate::config::ResetPolicy>,
    /// Human-facing details.
    pub summary: Option<Cow<'static, str>>,
}

/// Concrete record produced on each report() call, also logged.
/// Contains only runtime-mutable data; static configuration lives in FaultDescriptor.
#[derive(Debug, Clone)]
pub struct FaultRecord {
    pub fault_id: FaultId,
    pub time: SystemTime,
    pub severity: FaultSeverity,
    pub source: crate::ids::SourceId,
    pub lifecycle_phase: LifecyclePhase,
    pub stage: FaultLifecycleStage,
    pub environment_data: Vec<KeyValue>,
}

impl FaultRecord {
    /// Append environment data (mutable)
    pub fn add_environment_data(&mut self, key: &'static str, value: String) {
        self.environment_data.push(KeyValue { key, value });
        self.time = SystemTime::now();
    }

    /// Update lifecycle stage (mutable)
    pub fn update_stage(&mut self, stage: FaultLifecycleStage) {
        self.stage = stage;
        self.time = SystemTime::now();
    }

    /// Update severity (mutable)
    pub fn update_severity(&mut self, severity: FaultSeverity) {
        self.severity = severity;
        self.time = SystemTime::now();
    }

}
