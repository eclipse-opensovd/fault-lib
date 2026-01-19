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
use crate::reporter::ReporterConfig;
use crate::utils::*;
use common::config::ResetPolicy;
use common::debounce::DebounceMode;
use common::fault::*;
use common::ids::*;
use common::types::*;
use std::string::String;
use std::time::Duration;

#[allow(dead_code)]
pub fn stub_source() -> SourceId {
    SourceId {
        entity: to_static_short_string("source").unwrap(),
        ecu: Some(ShortString::from_bytes("ECU-A".as_bytes()).unwrap()),
        domain: Some(to_static_short_string("ADAS").unwrap()),
        sw_component: Some(to_static_short_string("Perception").unwrap()),
        instance: Some(to_static_short_string("0").unwrap()),
    }
}

#[allow(dead_code)]
pub fn stub_config() -> ReporterConfig {
    ReporterConfig {
        source: stub_source(),
        lifecycle_phase: LifecyclePhase::Running,
        default_env_data: MetadataVec::new(),
    }
}

#[allow(dead_code)]
pub fn stub_descriptor(id: FaultId, name: ShortString, debounce: Option<DebounceMode>, reset: Option<ResetPolicy>) -> FaultDescriptor {
    FaultDescriptor {
        id,
        name,
        summary: None,
        category: FaultType::Software,
        severity: FaultSeverity::Warn,
        compliance: ComplianceVec::new(),
        reporter_side_debounce: debounce,
        reporter_side_reset: reset,
        manager_side_debounce: None,
        manager_side_reset: None,
    }
}

#[allow(dead_code)]
pub fn stub_record(desc: FaultDescriptor) -> FaultRecord {
    FaultRecord {
        id: desc.id,
        time: IpcTimestamp::default(),
        source: stub_source(),
        lifecycle_phase: LifecyclePhase::Running,
        lifecycle_stage: LifecycleStage::NotTested,
        env_data: MetadataVec::new(),
    }
}

pub fn create_dummy_descriptors() -> Vec<FaultDescriptor> {
    let d1 = FaultDescriptor {
        id: FaultId::Text(to_static_short_string("d1").unwrap()),

        name: to_static_short_string("Descriptor 1").unwrap(),
        summary: None,

        category: FaultType::Software,
        severity: FaultSeverity::Debug,
        compliance: ComplianceVec::try_from(&[ComplianceTag::EmissionRelevant, ComplianceTag::SafetyCritical][..]).unwrap(),

        reporter_side_debounce: Some(DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100_u64),
        }),
        reporter_side_reset: None,
        manager_side_debounce: None,
        manager_side_reset: None,
    };

    let d2 = FaultDescriptor {
        id: FaultId::Text(to_static_short_string("d2").unwrap()),

        name: to_static_short_string("Descriptor 2").unwrap(),
        summary: Some(to_static_long_string("Human-readable summary").unwrap()),

        category: FaultType::Configuration,
        severity: FaultSeverity::Warn,
        compliance: ComplianceVec::try_from(&[ComplianceTag::SecurityRelevant, ComplianceTag::SafetyCritical][..]).unwrap(),

        reporter_side_debounce: None,
        reporter_side_reset: None,
        manager_side_debounce: Some(DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_millis(100_u64),
        }),
        manager_side_reset: None,
    };
    vec![d1, d2]
}

pub fn load_dummy_config_file() -> String {
    serde_json::to_string(&create_dummy_descriptors()).expect("serde_json::to_string failed")
}
