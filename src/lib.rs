#![forbid(unsafe_code)] // enforce safe Rust across the crate
// The public surface collects the building blocks for reporters, descriptors,
// and sinks so callers can just `use fault_lib::*` and go.
pub mod api;
pub mod catalog;
pub mod config;
pub mod ids;
pub mod model;
pub mod sink;
pub mod util;

// Re-export the main user-facing pieces, this keeps the crate ergonomic without
// forcing consumers to dig through modules.
pub use api::{FaultApi, Reporter};
pub use catalog::FaultCatalog;
pub use config::{DebouncePolicy, ResetPolicy, ReportOptions, ReporterConfig};
pub use ids::{FaultId, SourceId};
pub use model::{
    ComplianceTag, FaultDescriptor, FaultLifecycleStage, FaultRecord, FaultSeverity,
    FaultType, KeyValue, LifecyclePhase,
};
pub use sink::{FaultSink, LogHook};
