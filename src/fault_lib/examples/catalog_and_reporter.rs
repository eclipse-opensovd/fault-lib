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

use common::{
    SourceId,
    fault::{FaultId, LifecyclePhase, LifecycleStage},
    types::{MetadataVec, to_static_short_string},
};
use fault_lib::{
    FaultApi,
    catalog::FaultCatalogBuilder,
    reporter::{Reporter, ReporterApi, ReporterConfig},
};
use std::thread;

fn main() {
    let json = std::fs::read_to_string("src/fault_lib/tests/data/hvac_fault_catalog.json")
        .expect("catalog file");
    let _api = FaultApi::new(
        FaultCatalogBuilder::new()
            .json_string(&json)
            .expect("builder config")
            .build(),
    );

    let t1 = thread::spawn(move || {
        let source = SourceId {
            entity: to_static_short_string("entity1").unwrap(),
            ecu: Some(to_static_short_string("ecu").unwrap()),
            domain: Some(to_static_short_string("domain").unwrap()),
            sw_component: Some(to_static_short_string("component1").unwrap()),
            instance: Some(to_static_short_string("1").unwrap()),
        };
        let config = ReporterConfig {
            source,
            lifecycle_phase: LifecyclePhase::Running,
            default_env_data: MetadataVec::try_from(
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

        let mut reporter =
            Reporter::new(&FaultId::Numeric(0x7001), config).expect("get_descriptor failed");

        let record = reporter.create_record(LifecycleStage::Passed);

        let _ = reporter.publish("test/path", record);
    });

    let t2 = thread::spawn(move || {
        let source = SourceId {
            entity: to_static_short_string("entity2").unwrap(),
            ecu: Some(to_static_short_string("ecu").unwrap()),
            domain: Some(to_static_short_string("domain").unwrap()),
            sw_component: Some(to_static_short_string("component2").unwrap()),
            instance: Some(to_static_short_string("2").unwrap()),
        };
        let config = ReporterConfig {
            source,
            lifecycle_phase: LifecyclePhase::Running,
            default_env_data: MetadataVec::try_from(
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

        let mut reporter = Reporter::new(
            &FaultId::Text(to_static_short_string("hvac.blower.speed_sensor_mismatch").unwrap()),
            config,
        )
        .expect("get_descriptor failed");

        let record = reporter.create_record(LifecycleStage::Passed);

        let _ = reporter.publish("test/path", record);
    });

    let _ = t1.join();
    let _ = t2.join();
}
