//! Example only: illustrates how a vehicle component could wire up `fault-lib`.
//! This code is intentionally incomplete and is not meant to be built.

// Shared ownership (Arc) lets us pass the API handles around
// Duration keeps the debounce/reset numbers readable.
use std::sync::Arc;
use std::time::Duration;

// Pull the pieces the component needs: descriptor macro, catalog, policies,
// IDs, domain models, and the traits for logging/sink integration.
use fault_lib::{
    FaultRecord, Reporter,
    api::FaultApi,
    catalog::FaultCatalog,
    config::{
        DebounceMode, DebouncePolicy, ReportOptions, ReporterConfig, ResetPolicy, ResetTrigger,
    },
    fault_descriptor,
    ids::{FaultId, SourceId},
    model::{ComplianceTag, FaultSeverity, FaultType, KeyValue, LifecyclePhase},
    sink::{FaultSink, LogHook, SinkError},
};

/// Catalog slice: in a real code base this could be generated
/// so the component and DFM stay in sync about IDs and policies.
static HVAC_DESCRIPTORS: &[fault_lib::model::FaultDescriptor] = &[
    // `fault_descriptor!` is a small macro helper that expands to a struct literal.
    fault_descriptor! {
        id = FaultId::Numeric(0x7001),
        name = "CabinTempSensorStuck",
        kind = FaultType::Hardware,
        severity = FaultSeverity::Warn,
        compliance = [ComplianceTag::SafetyCritical],
        summary = "Cabin temperature sensor delivered the same sample for >60s",
        docs = "Cabin temperature sensor delivered the same sample for >60s",
        debounce = DebouncePolicy {
            mode: DebounceMode::HoldTime { duration: Duration::from_secs(60) },
            log_throttle: Some(Duration::from_secs(300)),
        },
        reset = ResetPolicy {
            trigger: ResetTrigger::StableFor(Duration::from_secs(900)),
            min_operating_cycles_before_clear: Some(5),
        }
    },
    fault_descriptor! {
        id = FaultId::Text("hvac.blower.speed_sensor_mismatch"),
        name = "BlowerSpeedMismatch",
        kind = FaultType::Communication,
        severity = FaultSeverity::Error,
        compliance = [ComplianceTag::EmissionRelevant],
        summary = "Commanded and measured blower speeds diverged beyond tolerance",
        docs = "Cabin temperature sensor delivered the same sample for >60s",
        debounce = DebouncePolicy {
            mode: DebounceMode::HoldTime { duration: Duration::from_secs(60) },
            log_throttle: Some(Duration::from_secs(300)),
        },
        reset = ResetPolicy {
            trigger: ResetTrigger::StableFor(Duration::from_secs(900)),
            min_operating_cycles_before_clear: Some(5),
        }
    },
];

/// Bundle descriptors with an identifier + version so the DFM can verify compatibility.
static HVAC_CATALOG: FaultCatalog = FaultCatalog::new("hvac", 3, HVAC_DESCRIPTORS);

/// Minimal log hook to keep the example focused on the API touchpoints.
struct StdoutLogHook;

impl LogHook for StdoutLogHook {
    fn on_report(&self, record: &fault_lib::model::FaultRecord) {
        println!(
            "[fault-log] {} severity={:?} source={}",
            record.descriptor.name, record.severity, record.source
        );
    }
}

/// Dummy sink used for illustration. Real code would forward to S-CORE IPC.
struct VehicleBusSink;

#[allow(clippy::unused_async)]
impl FaultSink for VehicleBusSink {
    // In real deployments this is where we would enqueue into IPC to the central manager.
    fn publish(&self, record: &fault_lib::model::FaultRecord) -> Result<(), SinkError> {
        println!(
            "[fault-sink] queued {} (catalog={}#{})",
            record.descriptor.name, record.catalog_id, record.catalog_version
        );
        Ok(())
    }
}

/// Components wire this during init and hold on to the `Reporter`.
fn init_hvac_faults() -> Reporter {
    // FaultApi owns the sink/logger/catalog. Arc makes cloning cheap for async closures.
    let api = FaultApi::new(
        Arc::new(VehicleBusSink),
        Arc::new(StdoutLogHook),
        Arc::new(HVAC_CATALOG.clone()),
    );

    // ReporterConfig carries static identity for this ECU/component plus any default metadata.
    let reporter_cfg = ReporterConfig {
        source: SourceId {
            entity: "HVAC.Controller",
            ecu: Some("CCU-SoC-A"),
            domain: Some("HVAC"),
            sw_component: Some("ClimateManager"),
            instance: None,
        },
        lifecycle_phase: LifecyclePhase::Running,
        default_meta: vec![KeyValue {
            key: "sw.version",
            value: "2024.10.0".into(),
        }],
    };

    api.reporter(reporter_cfg)
}

/// Somewhere in the control loop we can raise faults using the reporter.
#[allow(dead_code)]
// `async` because publishing may involve I/O; Rust futures make it cheap to await.
fn handle_blower_fault(reporter: &Reporter, measured_rpm: f32, commanded_rpm: f32) {
    // Look up the descriptor we registered earlier. Real code would likely keep
    // a direct reference instead of searching each time.
    let record: FaultRecord = FaultRecord::new(reporter, &FaultId::Text("hvac.blower.speed_senso"))
        .with_severity(None)
        .with_metadata("measured_rpm", measured_rpm.to_string())
        .with_metadata("commanded_rpm", commanded_rpm.to_string())
        .with_debounce(None)
        .with_reset(None);

    // The reporter logs locally, tags the record with catalog/version,
    // and hands it off to the sink for transport.
    if let Err(err) = reporter.report(&record) {
        eprintln!("failed to publish blower mismatch fault: {err}");
    }
}

#[test]
fn test_handle_blower_fault() {
    let reporter = init_hvac_faults();
    handle_blower_fault(&reporter, 0.6, 0.9);
}
