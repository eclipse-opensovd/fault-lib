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

use common::debounce;
use common::fault;
use dfm_lib::diagnostic_fault_manager::DiagnosticFaultManager;
use dfm_lib::fault_catalog_registry::*;
use dfm_lib::sovd_fault_manager::*;
use dfm_lib::sovd_fault_storage::*;
use env_logger::Env;
use fault_lib::catalog::{FaultCatalogBuilder, FaultCatalogConfig};
use fault_lib::utils::to_static_long_string;
use fault_lib::utils::to_static_short_string;
use std::time::Duration;
use tempfile::tempdir;

fn load_hvac_config() -> FaultCatalogConfig {
    let f1 = fault::FaultDescriptor {
        id: fault::FaultId::Numeric(0x7001),

        name: to_static_short_string("CabinTempSensorStuck").unwrap(),
        summary: None,

        category: fault::FaultType::Communication,
        severity: fault::FaultSeverity::Error,
        compliance: fault::ComplianceVec::try_from(&[fault::ComplianceTag::EmissionRelevant][..]).unwrap(),

        reporter_side_debounce: Some(debounce::DebounceMode::HoldTime {
            duration: Duration::from_secs(60),
        }),
        reporter_side_reset: None,
        manager_side_debounce: None,
        manager_side_reset: None,
    };

    let f2 = fault::FaultDescriptor {
        id: fault::FaultId::Text(to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap()),

        name: to_static_short_string("BlowerSpeedMismatch").unwrap(),
        summary: Some(to_static_long_string("Human-readable summary").unwrap()),

        category: fault::FaultType::Communication,
        severity: fault::FaultSeverity::Error,
        compliance: fault::ComplianceVec::try_from(&[fault::ComplianceTag::SecurityRelevant, fault::ComplianceTag::SafetyCritical][..]).unwrap(),

        reporter_side_debounce: None,
        reporter_side_reset: None,
        manager_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100_u64),
        }),
        manager_side_reset: None,
    };

    let faults = vec![f1, f2];
    FaultCatalogConfig {
        id: "hvac".into(),
        version: 3,
        faults,
    }

    // serde_json::to_string(&[d1, d2]).expect("serde_json::to_string failed")
}

fn load_ivi_config() -> FaultCatalogConfig {
    let f1 = fault::FaultDescriptor {
        id: fault::FaultId::Text(to_static_short_string("d1").unwrap()),

        name: to_static_short_string("Descriptor 1").unwrap(),
        summary: None,

        category: fault::FaultType::Software,
        severity: fault::FaultSeverity::Debug,
        compliance: fault::ComplianceVec::try_from(&[fault::ComplianceTag::EmissionRelevant, fault::ComplianceTag::SafetyCritical][..]).unwrap(),

        reporter_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100_u64),
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
        compliance: fault::ComplianceVec::try_from(&[fault::ComplianceTag::SecurityRelevant, fault::ComplianceTag::SafetyCritical][..]).unwrap(),

        reporter_side_debounce: None,
        reporter_side_reset: None,
        manager_side_debounce: Some(debounce::DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100_u64),
        }),
        manager_side_reset: None,
    };

    let faults = vec![f1, f2];
    FaultCatalogConfig {
        id: "ivi".into(),
        version: 1,
        faults,
    }

    // serde_json::to_string(&[d1, d2]).expect("serde_json::to_string failed")
}
fn main() {
    let env = Env::default().filter_or("RUST_LOG", "debug");
    env_logger::init_from_env(env);

    let storage_dir = tempdir().unwrap();
    let storage = KvsSovdFaultStateStorage::new(storage_dir.path(), 0).unwrap();

    let hvac_catalog = FaultCatalogBuilder::new().cfg_struct(load_hvac_config()).build();
    let ivi_catalog = FaultCatalogBuilder::new().cfg_struct(load_ivi_config()).build();

    let registry = FaultCatalogRegistry::new(vec![hvac_catalog, ivi_catalog]);

    let dfm = DiagnosticFaultManager::new(storage, registry);
    let manager = dfm.get_sovd_fault_manager();

    // Try to get faults for a non-existent path.
    let faults = manager.get_all_faults("invalid_hvac");
    assert!(faults.is_err());
    assert_eq!(faults.unwrap_err(), Error::BadArgument);

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{:?}", faults);

    /*
    let record = fault::FaultRecord {
        id: fault::FaultId::Text(to_static_short_string("d1").unwrap()),
        time: IpcTimestamp::default(),
        source: SourceId {
            entity: to_static_short_string("source").unwrap(),
            ecu: Some(ShortString::from_bytes("ECU-A".as_bytes()).unwrap()),
            domain: Some(to_static_short_string("ADAS").unwrap()),
            sw_component: Some(to_static_short_string("Perception").unwrap()),
            instance: Some(to_static_short_string("0").unwrap()),
        },
        lifecycle_phase: fault::LifecyclePhase::Running,
        lifecycle_stage: fault::LifecycleStage::Failed,
        env_data: MetadataVec::try_from(
            &[
                (to_static_short_string("k1").unwrap(), to_static_short_string("v1").unwrap()),
                (to_static_short_string("k2").unwrap(), to_static_short_string("v2").unwrap()),
            ][..],
        )
        .unwrap(),
    };

    // TODO: Send record via app here
    */

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{:?}", faults);

    let fault = manager.get_fault("hvac", &faults[0].code).unwrap();
    println!("{:?}", fault);
}
