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
use crate::{debounce::DebouncePolicy, fault::ComplianceVec, fault::FaultSeverity, types::*};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use iceoryx2::prelude::*;
use iceoryx2_bb_container::vector::*;

// Reset rules define how and when a latched fault can be cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum ResetTrigger {
    /// Clear on next ignition/power cycle count meeting threshold.
    PowerCycles(u32),
    /// Clear when condition absent for a duration.
    StableFor(Duration),
    /// Manual maintenance/tooling only (e.g., regulatory).
    ToolOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub struct ResetPolicy {
    pub trigger: ResetTrigger,
    /// Some regulations require X cycles before clearable from user UI.
    pub min_operating_cycles_before_clear: Option<u32>,
}

// Per-report options provided by the call site when a fault is emitted.
#[derive(Debug, Default, Clone, ZeroCopySend)]
#[repr(C)]
pub struct ReportOptions {
    /// Override severity (else descriptor.default_severity).
    pub severity: Option<FaultSeverity>,
    /// Attach extra metadata key-values (free form).
    pub metadata: StaticVec<(ShortString, ShortString), 8>,
    /// Override policies dynamically (rare, but useful for debug/A-B).
    pub debounce: Option<DebouncePolicy>,
    pub reset: Option<ResetPolicy>,
    /// Regulatory/operational flags—extra tags may be added at report time.
    pub extra_compliance: ComplianceVec,
}
