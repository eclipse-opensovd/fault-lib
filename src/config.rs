use std::time::Duration;

// Debounce descriptions capture how noisy fault sources should be filtered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebounceMode {
    /// Require N occurrences within a window to confirm fault Active.
    CountWithinWindow { min_count: u32, window: Duration },
    /// Confirm when signal remains bad for duration (e.g., stuck-at).
    HoldTime { duration: Duration },
    /// Edge triggered (first occurrence) with cooldown to avoid flapping.
    EdgeWithCooldown { cooldown: Duration },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebouncePolicy {
    pub mode: DebounceMode,
    /// Optional suppression of repeats in logging within a time window.
    pub log_throttle: Option<Duration>,
}

// Reset rules define how and when a latched fault can be cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetTrigger {
    /// Clear on next ignition/power cycle count meeting threshold.
    PowerCycles(u32),
    /// Clear when condition absent for a duration.
    StableFor(Duration),
    /// Manual maintenance/tooling only (e.g., regulatory).
    ToolOnly,
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
    pub default_meta: Vec<crate::model::KeyValue>,
}

// Per-report options provided by the call site when a fault is emitted.
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Override severity (else descriptor.default_severity).
    pub severity: Option<crate::model::FaultSeverity>,
    /// Attach extra metadata key-values (free form).
    pub metadata: Vec<crate::model::KeyValue>,
    /// Override policies dynamically (rare, but useful for debug/A-B).
    pub debounce: Option<DebouncePolicy>,
    pub reset: Option<ResetPolicy>,
    /// Regulatory/operational flags—extra tags may be added at report time.
    pub extra_compliance: Vec<crate::model::ComplianceTag>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            severity: None,
            metadata: vec![],
            debounce: None,
            reset: None,
            extra_compliance: vec![],
        }
    }
}
