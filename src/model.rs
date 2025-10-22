use std::time::SystemTime;

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

/// State of a fault’s lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultLifecycleStage {
    Raised,      // newly observed; debounce may still be in progress
    Active,      // confirmed and visible to the system/user
    Latched,     // sticky until reset policy conditions met
    Cleared,     // condition gone; record maintained per policy
}

/// Minimal, typed metadata; keep serde-agnostic at the API edge.
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
    pub name: &'static str,
    pub fault_type: FaultType,
    pub default_severity: FaultSeverity,
    pub compliance: &'static [ComplianceTag],
    /// Default debounce/reset; can be overridden per-report via ReportOptions.
    pub debounce: Option<crate::config::DebouncePolicy>,
    pub reset: Option<crate::config::ResetPolicy>,
    /// Human-facing details.
    pub summary: Option<&'static str>,
    pub docs_url: Option<&'static str>,
}

/// Concrete record produced on each report() call, also logged.
#[derive(Debug, Clone)]
pub struct FaultRecord {
    pub time: SystemTime,
    pub descriptor: FaultDescriptor,
    pub severity: FaultSeverity,
    pub source: crate::ids::SourceId,
    pub lifecycle_phase: LifecyclePhase,
    pub metadata: Vec<KeyValue>,
    pub catalog_id: &'static str,
    pub catalog_version: u64,
    pub compliance: Vec<ComplianceTag>,
    pub effective_debounce: Option<crate::config::DebouncePolicy>,
    pub effective_reset: Option<crate::config::ResetPolicy>,
}
