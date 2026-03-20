/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 */
#![allow(clippy::unwrap_used, clippy::expect_used)]

use core::time::Duration;

use common::{
    catalog::{FaultCatalogBuilder, FaultCatalogConfig},
    debounce, fault,
    types::{to_static_long_string, to_static_short_string},
};
use dfm_lib::{
    diagnostic_fault_manager::DiagnosticFaultManager, fault_catalog_registry::FaultCatalogRegistry,
    sovd_fault_manager::Error, sovd_fault_storage::KvsSovdFaultStateStorage,
};
use tempfile::tempdir;
use tracing_subscriber::EnvFilter;

fn load_hvac_config() -> FaultCatalogConfig {
    let f1 = fault::FaultDescriptor {
        id: fault::FaultId::Numeric(0x7001),

        name: to_static_short_string("CabinTempSensorStuck").unwrap(),
        summary: None,

        category: fault::FaultType::Communication,
        severity: fault::FaultSeverity::Error,
        compliance: fault::ComplianceVec::try_from(&[fault::ComplianceTag::EmissionRelevant][..])
            .unwrap(),

        reporter_side_debounce: Some(debounce::DebounceMode::HoldTime {
            duration: Duration::from_secs(60).into(),
        }),
        reporter_side_reset: None,
        manager_side_debounce: None,
        manager_side_reset: None,
    };

    let f2 = fault::FaultDescriptor {
        id: fault::FaultId::Text(
            to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap(),
        ),

        name: to_static_short_string("BlowerSpeedMismatch").unwrap(),
        summary: Some(to_static_long_string("Human-readable summary").unwrap()),

        category: fault::FaultType::Communication,
        severity: fault::FaultSeverity::Error,
        compliance: fault::ComplianceVec::try_from(
            &[
                fault::ComplianceTag::SecurityRelevant,
                fault::ComplianceTag::SafetyCritical,
            ][..],
        )
        .unwrap(),

        reporter_side_debounce: None,
        reporter_side_reset: None,
        manager_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100u64).into(),
        }),
        manager_side_reset: None,
    };

    let faults = vec![f1, f2];
    FaultCatalogConfig {
        id: "hvac".into(),
        version: 3,
        faults,
    }
}

fn load_ivi_config() -> FaultCatalogConfig {
    let f1 = fault::FaultDescriptor {
        id: fault::FaultId::Text(to_static_short_string("d1").unwrap()),

        name: to_static_short_string("Descriptor 1").unwrap(),
        summary: None,

        category: fault::FaultType::Software,
        severity: fault::FaultSeverity::Debug,
        compliance: fault::ComplianceVec::try_from(
            &[
                fault::ComplianceTag::EmissionRelevant,
                fault::ComplianceTag::SafetyCritical,
            ][..],
        )
        .unwrap(),

        reporter_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100u64).into(),
        }),
        reporter_side_reset: None,
        manager_side_debounce: None,
        manager_side_reset: None,
    };

    let f2 = fault::FaultDescriptor {
        id: fault::FaultId::Text(to_static_short_string("d2").unwrap()),

        name: to_static_short_string("Descriptor 2").unwrap(),
        summary: Some(to_static_long_string("Human-readable summary").unwrap()),

        category: fault::FaultType::Configuration,
        severity: fault::FaultSeverity::Warn,
        compliance: fault::ComplianceVec::try_from(
            &[
                fault::ComplianceTag::SecurityRelevant,
                fault::ComplianceTag::SafetyCritical,
            ][..],
        )
        .unwrap(),

        reporter_side_debounce: None,
        reporter_side_reset: None,
        manager_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100u64).into(),
        }),
        manager_side_reset: None,
    };

    let faults = vec![f1, f2];
    FaultCatalogConfig {
        id: "ivi".into(),
        version: 1,
        faults,
    }
}
#[allow(clippy::indexing_slicing)]
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "debug".into()))
        .init();

    let storage_dir = tempdir().unwrap();
    let storage = KvsSovdFaultStateStorage::new(storage_dir.path(), 0).expect("storage init");

    let hvac_catalog = FaultCatalogBuilder::new()
        .cfg_struct(load_hvac_config())
        .expect("builder config")
        .build();
    let ivi_catalog = FaultCatalogBuilder::new()
        .cfg_struct(load_ivi_config())
        .expect("builder config")
        .build();

    let registry = FaultCatalogRegistry::new(vec![hvac_catalog, ivi_catalog]);

    let dfm = DiagnosticFaultManager::new(storage, registry);
    let manager = dfm.create_sovd_fault_manager();

    // Try to get faults for a non-existent path.
    let faults = manager.get_all_faults("invalid_hvac");
    assert!(faults.is_err());
    assert_eq!(faults.unwrap_err(), Error::BadArgument);

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{faults:?}");

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{faults:?}");

    let fault = manager.get_fault("hvac", &faults[0].code).unwrap();
    println!("{fault:?}");
}
