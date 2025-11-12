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

use std::time::Duration;

// Debounce descriptions capture how noisy fault sources should be filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebounceMode {
    /// Require N occurrences within a window to confirm fault Failed (transition PreFailed -> Failed).
    CountWithinWindow { min_count: u32, window: Duration },
    /// Confirm when condition remains continuously bad for at least `duration`.
    /// Use for stuck-at / persistent faults where transient glitches should be ignored.
    /// Example: sensor delivers identical reading for 60s -> `HoldTime { duration: Duration::from_secs(60) }`.
    HoldTime { duration: Duration },
    /// Trigger immediately on first occurrence, then suppress further activations until the cooldown elapses.
    /// Use for faults that are meaningful on first edge but may flap rapidly.
    /// Example: first CAN bus-off event activates fault, ignore subsequent bus-off transitions for 5s -> `EdgeWithCooldown { cooldown: Duration::from_secs(5) }`.
    EdgeWithCooldown { cooldown: Duration },
    /// Pure count based: confirm after total (cumulative) occurrences reach threshold.
    /// Useful for sporadic errors where temporal proximity is less important than frequency.
    /// Example: activate after 10 checksum mismatches regardless of timing.
    CountThreshold { min_count: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebouncePolicy {
    pub mode: DebounceMode,
    /// Optional suppression of repeats in logging within a time window.
    pub log_throttle: Option<Duration>,
}

// Reset rules define how and when a confirmed (Failed) test result transitions back to Passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetTrigger {
    /// Clear when a given operation cycle kind count meets threshold (e.g. ignition, drive, charge).
    /// `cycle_ref` is a symbolic identifier (e.g. "ignition.main", "drive.standard") allowing
    /// the DFM to correlate with its cycle counter source.
    OperationCycles { kind: OperationCycleKind, min_cycles: u32, cycle_ref: &'static str },
    /// Clear after the fault condition has been continuously absent (tests passing) for `duration`.
    /// Relation to cycles: If the reset must align to authoritative operation cycle boundaries, choose
    /// `OperationCycles`; `StableFor` is wall/time-source based (monotonic) and independent of cycle counting.
    StableFor(Duration),
    /// Manual maintenance/tooling only (e.g., regulatory).
    DiagnosticTester,
}

/// Enumerates common operation cycle archetypes relevant for aging/reset semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationCycleKind {
    Ignition,   // Traditional ignition/power cycle
    Drive,      // Complete drive cycle (start -> run -> stop)
    Charge,     // Entire HV battery charge session
    Thermal,    // HVAC or thermal management cycle
    Custom(&'static str), // Domain specific cycle identifier
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPolicy {
    pub trigger: ResetTrigger,
    /// Some regulations require X cycles before clearable from user UI.
    pub min_operating_cycles_before_clear: Option<u32>,
}

// Per-component defaults that get baked into a Reporter instance.
#[derive(Debug, Clone)]
pub struct ReporterConfig {
    pub source: crate::ids::SourceId,
    pub lifecycle_phase: crate::model::LifecyclePhase,
    /// Optional per-reporter defaults (e.g., common metadata).
    pub default_environment_data: Vec<crate::model::KeyValue>,
}

// Per-report options provided by the call site when a fault is emitted.
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    /// Override severity (else descriptor.default_severity).
    pub severity: Option<crate::model::FaultSeverity>,
    /// Attach extra metadata key-values (free form).
    pub environment_data: Vec<crate::model::KeyValue>,
    /// Override policies dynamically (rare, but useful for debug/A-B).
    pub debounce: Option<DebouncePolicy>,
    pub reset: Option<ResetPolicy>,
    /// Regulatory/operational flags—extra tags may be added at report time.
    pub extra_compliance: Vec<crate::model::ComplianceTag>,
}
