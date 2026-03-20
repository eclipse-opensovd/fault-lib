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
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::std_instead_of_alloc)]

use core::time::Duration;
use std::sync::Arc;

use common::{
    SourceId,
    catalog::{FaultCatalogBuilder, FaultCatalogConfig},
    debounce, fault,
    fault::IpcTimestamp,
    types::{MetadataVec, ShortString, to_static_long_string, to_static_short_string},
};
use dfm_lib::{
    OperationCycleTracker,
    fault_catalog_registry::FaultCatalogRegistry,
    fault_record_processor::FaultRecordProcessor,
    sovd_fault_manager::{Error, SovdFaultManager},
    sovd_fault_storage::KvsSovdFaultStateStorage,
};
use tempfile::tempdir;

fn load_config_file() -> FaultCatalogConfig {
    let d1 = fault::FaultDescriptor {
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

    let d2 = fault::FaultDescriptor {
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
    let faults = vec![d1, d2];
    FaultCatalogConfig {
        id: "hvac".into(),
        version: 3,
        faults,
    }
}

#[allow(clippy::indexing_slicing)]
fn main() {
    let storage_dir = tempdir().unwrap();

    let storage =
        Arc::new(KvsSovdFaultStateStorage::new(storage_dir.path(), 0).expect("storage init"));

    let cfg = load_config_file();
    let registry = Arc::new(FaultCatalogRegistry::new(vec![
        FaultCatalogBuilder::new()
            .cfg_struct(cfg)
            .expect("builder config")
            .build(),
    ]));

    let cycle_tracker = Arc::new(std::sync::RwLock::new(OperationCycleTracker::new()));
    let mut processor =
        FaultRecordProcessor::new(Arc::clone(&storage), Arc::clone(&registry), cycle_tracker);
    let manager = SovdFaultManager::new(storage, registry);

    // Try to get faults for a non-existent path.
    let faults = manager.get_all_faults("invalid_hvac");
    assert!(faults.is_err());
    assert_eq!(faults.unwrap_err(), Error::BadArgument);

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{faults:?}");

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
                (
                    to_static_short_string("k1").unwrap(),
                    to_static_short_string("v1").unwrap(),
                ),
                (
                    to_static_short_string("k2").unwrap(),
                    to_static_short_string("v2").unwrap(),
                ),
            ][..],
        )
        .unwrap(),
    };

    processor.process_record(&to_static_long_string("hvac").unwrap(), &record);

    let faults = manager.get_all_faults("hvac").unwrap();
    println!("{faults:?}");

    let fault = manager.get_fault("hvac", &faults[0].code).unwrap();
    println!("{fault:?}");
}
