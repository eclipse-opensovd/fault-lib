use crate::FaultId;
use crate::{DebouncePolicy, Reporter};
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
    Raised,  // newly observed; debounce may still be in progress
    Active,  // confirmed and visible to the system/user
    Latched, // sticky until reset policy conditions met
    Cleared, // condition gone; record maintained per policy
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

impl FaultRecord {
    pub fn new(reporter: &Reporter, descriptor_key: &FaultId) -> Self {
        let descriptor = reporter
            .catalog()
            .find(descriptor_key)
            .expect("descriptor must exist in catalog");
        Self {
            time: SystemTime::now(),
            descriptor: descriptor.clone(),
            severity: descriptor.default_severity,
            source: reporter.cfg().source.clone(),
            lifecycle_phase: reporter.cfg().lifecycle_phase,
            metadata: reporter.cfg().default_meta.clone(),
            catalog_id: reporter.catalog().id,
            catalog_version: reporter.catalog().version,
            compliance: descriptor.compliance.to_vec(),
            effective_debounce: descriptor.debounce.clone(),
            effective_reset: descriptor.reset.clone(),
        }
    }

    pub fn with_debounce(self, debounce: Option<DebouncePolicy>) -> Self {
        let mut req = self;
        req.effective_debounce = debounce.or_else(|| req.descriptor.debounce.clone());
        req
    }

    pub fn with_extra_compliance(self, extra_compliance: Vec<ComplianceTag>) -> Self {
        let mut req = self;
        req.compliance.extend(extra_compliance);
        req
    }
    pub fn with_metadata(self, key: &'static str, value: String) -> Self {
        let mut req = self;
        req.metadata.push(KeyValue { key, value });
        req
    }
    pub fn with_reset(self, reset: Option<crate::ResetPolicy>) -> Self {
        let mut req = self;
        req.effective_reset = reset.or_else(|| req.descriptor.reset.clone());
        req
    }
    pub fn with_severity(self, fault_severity: Option<FaultSeverity>) -> Self {
        let mut req = self;
        req.severity = fault_severity.unwrap_or(req.descriptor.default_severity);
        req
    }
}
