/*
* Copyright (c) 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
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

//! **DESIGN REFERENCE** — illustrates the fault-lib API surface described in
//! `docs/design/design.md`.  All types and method signatures match the current
//! implementation.
//!
//! This file is **not** compiled as part of any workspace crate.  It serves as
//! living documentation that is kept in sync with the actual API.

use std::sync::Arc;
use core::time::Duration;

// --- common crate types ---
use common::catalog::{FaultCatalogBuilder, FaultCatalogConfig};
use common::config::ResetPolicy;
use common::debounce::DebounceMode;
use common::fault::*;
use common::ids::SourceId;
use common::sink_error::SinkError;
use common::types::*;

// --- fault_lib crate API ---
use fault_lib::FaultApi;
use fault_lib::reporter::{Reporter, ReporterApi, ReporterConfig};
use fault_lib::sink::LogHook;

// ============================================================================
// 1. Build a FaultCatalog from a FaultCatalogConfig
// ============================================================================
//
// In production the catalog is typically loaded from a JSON file via
// `FaultCatalogBuilder::new().json_file(path)?.build()`.  Here we construct
// the config in code for clarity.

fn build_hvac_catalog() -> common::catalog::FaultCatalog {
    let config = FaultCatalogConfig {
        id: "hvac".into(),
        version: 3,
        faults: vec![
            FaultDescriptor {
                id: FaultId::Numeric(0x7001),
                name: to_static_short_string("CabinTempSensorStuck").unwrap(),
                summary: Some(to_static_long_string(
                    "Cabin temperature sensor delivered the same sample for >60 s",
                ).unwrap()),
                category: FaultType::Hardware,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::try_from(
                    &[ComplianceTag::SafetyCritical][..],
                ).unwrap(),
                reporter_side_debounce: Some(DebounceMode::HoldTime {
                    duration: Duration::from_secs(60).into(),
                }),
                reporter_side_reset: Some(ResetPolicy {
                    trigger: common::config::ResetTrigger::StableFor(
                        Duration::from_secs(900).into(),
                    ),
                    min_operating_cycles_before_clear: Some(5),
                }),
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string(
                    "hvac.blower.speed_mismatch",
                ).unwrap()),
                name: to_static_short_string("BlowerSpeedMismatch").unwrap(),
                summary: Some(to_static_long_string(
                    "Commanded and measured blower speeds diverged beyond tolerance",
                ).unwrap()),
                category: FaultType::Communication,
                severity: FaultSeverity::Error,
                compliance: ComplianceVec::try_from(
                    &[ComplianceTag::EmissionRelevant][..],
                ).unwrap(),
                reporter_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(500).into(),
                }),
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
        ],
    };

    FaultCatalogBuilder::new()
        .cfg_struct(config)
        .expect("builder input")
        .build()
}

// ============================================================================
// 2. LogHook — observability bridge to your logging stack
// ============================================================================

/// Minimal log hook.  Real implementations would forward to DLT, tracing, etc.
struct StdoutLogHook;

impl LogHook for StdoutLogHook {
    fn on_publish(&self, record: &FaultRecord) {
        println!(
            "[fault-log] id={:?} stage={:?} source={}",
            record.id, record.lifecycle_stage, record.source,
        );
    }

    fn on_error(&self, record: &FaultRecord, error: &SinkError) {
        eprintln!(
            "[fault-log] FAILED id={:?} error={error}",
            record.id,
        );
    }
}

// ============================================================================
// 3. Application wiring: one Reporter per fault ID
// ============================================================================

struct HvacApp {
    #[allow(dead_code)]
    temp_sensor_fault: Reporter,
    blower_fault: Reporter,
}

impl HvacApp {
    /// Bind reporters at startup.
    ///
    /// `FaultApi` must be initialised before calling `Reporter::new`;
    /// each reporter looks up its descriptor in the global catalog and
    /// obtains a handle to the IPC sink.
    pub fn new(reporter_cfg: ReporterConfig) -> Self {
        Self {
            temp_sensor_fault: Reporter::new(
                &FaultId::Numeric(0x7001),
                reporter_cfg.clone(),
            )
            .expect("descriptor 0x7001 must exist in catalog"),

            blower_fault: Reporter::new(
                &FaultId::Text(
                    to_static_short_string("hvac.blower.speed_mismatch").unwrap(),
                ),
                reporter_cfg,
            )
            .expect("descriptor 'hvac.blower.speed_mismatch' must exist in catalog"),
        }
    }

    /// Simulate a control-loop iteration.
    pub fn step(&mut self) {
        self.handle_blower_fault(0.6, 0.9);
    }

    /// 4. At runtime: create a record, set lifecycle stage, publish.
    ///
    /// `create_record` captures the current timestamp.
    /// `publish` enqueues the record to the IPC sink (non-blocking).
    fn handle_blower_fault(&mut self, measured_rpm: f32, commanded_rpm: f32) {
        let _measured = measured_rpm;
        let _commanded = commanded_rpm;

        // Create a record stamped with the current wall-clock time.
        // The lifecycle stage (Failed / Passed / …) is set at creation.
        let record = self.blower_fault.create_record(LifecycleStage::Failed);

        // Publish to DFM via the IPC sink.
        // `path` identifies the IPC channel (e.g. service name).
        if let Err(err) = self.blower_fault.publish("hvac/blower", record) {
            eprintln!("failed to enqueue blower mismatch fault: {err}");
        }
    }
}

// ============================================================================
// Putting it all together
// ============================================================================

#[allow(dead_code)]
fn main() {
    // --- Startup ---

    // 1. Build the catalog (from config struct, JSON string, or JSON file).
    let catalog = build_hvac_catalog();

    // 2. Initialise the global FaultApi singleton (creates IPC sink).
    //    Must happen exactly once before any Reporter is created.
    let _api = FaultApi::new(catalog);

    // 3. (Optional) Register a log hook for observability.
    FaultApi::set_log_hook(Arc::new(StdoutLogHook)).ok();

    // 4. Create per-component ReporterConfig.
    let reporter_cfg = ReporterConfig {
        source: SourceId {
            entity: to_static_short_string("HVAC.Controller").unwrap(),
            ecu: Some(ShortString::from_bytes(b"CCU-SoC-A").unwrap()),
            domain: Some(to_static_short_string("HVAC").unwrap()),
            sw_component: Some(to_static_short_string("ClimateManager").unwrap()),
            instance: None,
        },
        lifecycle_phase: LifecyclePhase::Running,
        default_env_data: MetadataVec::new(),
    };

    // 5. Create the application with bound reporters.
    let mut app = HvacApp::new(reporter_cfg);

    // --- Runtime ---
    app.step();
}
