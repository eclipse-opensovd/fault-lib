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

//! Example only: illustrates how a vehicle component could wire up `fault-lib`.
//! This code is intentionally incomplete and is not meant to be built.

// Shared ownership (Arc) lets us pass the API handles around
// Duration keeps the debounce/reset numbers readable.
use std::sync::Arc;
use std::time::Duration;

// --- FAULT-LIB API USAGE PATTERN EXAMPLE ---
// This test demonstrates the recommended usage pattern for the fault-lib API.
// 1. Define static fault descriptors and catalog at compile time.
// 2. At startup, create one Reporter per fault ID (binding config, catalog, and API).
// 3. At runtime, create a mutable FaultRecord from the bound Reporter, update state, and publish.
// 4. Only runtime data is sent; static config is referenced via the Reporter.
use fault_lib::{
    Reporter, // Per-fault binding: one instance per fault ID
    api::FaultApi, // Global API handle: owns sink and logger
    catalog::FaultCatalog, // Static catalog of all descriptors
    config::{DebounceMode, DebouncePolicy, ReporterConfig, ResetPolicy, ResetTrigger},
    fault_descriptor, // Macro for concise descriptor definition
    ids::{FaultId, SourceId},
    model::{
        ComplianceTag, FaultLifecycleStage, FaultSeverity, FaultType, KeyValue, LifecyclePhase,
    },
    sink::{FaultSink, LogHook, SinkError},
};

/// 1. Define static fault descriptors and catalog at compile time.
/// In a real code base this could be generated so the component and DFM stay in sync about IDs and policies.
static HVAC_DESCRIPTORS: &[fault_lib::model::FaultDescriptor] = &[
    // `fault_descriptor!` is a small macro helper that expands to a struct literal.
    fault_descriptor! {
        id = FaultId::Numeric(0x7001),
        name = "CabinTempSensorStuck",
        kind = FaultType::Hardware,
        severity = FaultSeverity::Warn,
        compliance = [ComplianceTag::SafetyCritical],
        summary = "Cabin temperature sensor delivered the same sample for >60s",
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
        id = FaultId::text_const("hvac.blower.speed_sensor_mismatch"),
        name = "BlowerSpeedMismatch",
        kind = FaultType::Communication,
        severity = FaultSeverity::Error,
        compliance = [ComplianceTag::EmissionRelevant],
        summary = "Commanded and measured blower speeds diverged beyond tolerance",
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
/// In production, this would forward to a logging backend.
struct StdoutLogHook;

impl LogHook for StdoutLogHook {
    fn on_report(&self, record: &fault_lib::model::FaultRecord) {
        println!(
            "[fault-log] fault_id={:?} severity={:?} source={}",
            record.fault_id, record.severity, record.source
        );
    }
}

/// Dummy sink used for illustration. Real code would forward to S-CORE IPC or another transport.
struct VehicleBusSink;

impl FaultSink for VehicleBusSink {
    // In real deployments this is where we would enqueue into IPC to the central manager.
    fn publish(&self, record: &fault_lib::model::FaultRecord) -> Result<(), SinkError> {
        println!(
            "[fault-sink] queued fault_id={:?}",
            record.fault_id
        );
        Ok(())
    }
}

/// 2. At startup, create one Reporter per fault ID (binding config, catalog, and API).
/// Each Reporter is bound to a single fault and holds all static config for that fault.
struct DummyApp {
    #[allow(dead_code)]
    temp_sensor_fault: Reporter,
    blower_fault: Reporter,
}

impl DummyApp {
    /// Bind all reporters to their respective fault IDs at startup.
    /// This ensures type safety and avoids runtime lookups.
    /// It also can ensure that catalogue in app and DFM match.
    pub fn new(
        reporter_cfg: ReporterConfig,
        catalog: &FaultCatalog,
    ) -> Self {
        Self {
            temp_sensor_fault: Reporter::new(
                catalog,
                reporter_cfg.clone(),
                &FaultId::Numeric(0x7001),
            ),
            blower_fault: Reporter::new(
                catalog,
                reporter_cfg,
                &FaultId::text("hvac.blower.speed_sensor_mismatch"),
            ),
        }
    }

    /// Simulate a control loop step that may raise a fault.
    pub fn step(&self) {
        self.handle_blower_fault(0.6, 0.9);
    }

    /// 3. At runtime, create a mutable FaultRecord from the bound Reporter, update state, and publish.
    /// This pattern ensures only runtime data is sent; static config is referenced via the Reporter.
    #[allow(dead_code)]
    fn handle_blower_fault(&self, measured_rpm: f32, commanded_rpm: f32) {
        // Create a new record for this fault occurrence
        let mut record = self.blower_fault.create_record();
    // Attach runtime environment data
    record.add_environment_data("measured_rpm", measured_rpm.to_string());
    record.add_environment_data("commanded_rpm", commanded_rpm.to_string());
    // Mark test result as failed (confirmed after any debounce logic) for this occurrence
    record.update_stage(FaultLifecycleStage::Failed);

        // Publish the record via the bound reporter.
        // This enqueues the record to the configured FaultSink (IPC)
        // and is non-blocking for the caller (does not wait for DFM response).
        if let Err(err) = self.blower_fault.publish(&record) {
            eprintln!("failed to enqueue blower mismatch fault: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Components wire this during init and hold on to the `Reporter`.
    #[test]
    fn test_hvac_faults_with_dummy_app() {
        // 0. Setup: create the global FaultApi (owns sink/logger)
        // Initialize singleton FaultApi (sink + logger registered globally)
        let _api = FaultApi::new(
            Arc::new(VehicleBusSink),
            Arc::new(StdoutLogHook),
        );

        // 1. Setup: create the per-component ReporterConfig
        let reporter_cfg = ReporterConfig {
            source: SourceId {
                entity: "HVAC.Controller",
                ecu: Some("CCU-SoC-A"),
                domain: Some("HVAC"),
                sw_component: Some("ClimateManager"),
                instance: None,
            },
            lifecycle_phase: LifecyclePhase::Running,
            default_environment_data: vec![KeyValue {
                key: "sw.version",
                value: "2024.10.0".into(),
            }],
        };

        // 2. Bind all reporters to their respective fault IDs at startup
    let dummy_app = DummyApp::new(reporter_cfg, &HVAC_CATALOG);

        // 3. Simulate a control loop step that may raise a fault
        dummy_app.step();
    }
}
