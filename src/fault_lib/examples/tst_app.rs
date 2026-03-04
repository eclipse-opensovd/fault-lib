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

use clap::Parser;
use common::fault::*;
use common::types::MetadataVec;
use fault_lib::FaultApi;
use fault_lib::catalog::FaultCatalogBuilder;
use tracing_subscriber::EnvFilter;

use fault_lib::reporter::Reporter;
use fault_lib::reporter::ReporterApi;
use fault_lib::reporter::ReporterConfig;
use fault_lib::utils::to_static_short_string;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tracing::*;

/// Command line arguments
#[derive(Parser, Debug)]
#[command(version, about, long_about= None)]
struct Args {
    /// path to fault catalog json file  
    #[arg(short, long)]
    config_file: PathBuf,
}

fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "debug".into()))
        .init();
    info!("Start Basic fault library example");
    // Create the FaultLib API object. We have to create it before any Fault API can be used
    // and keep it on stack until end of the program. No need to hand it over somewhere
    let _api = FaultApi::new(
        FaultCatalogBuilder::new()
            .json_file(args.config_file)
            .expect("builder config")
            .build(),
    );

    // here you can use any public api from fault-api
    playground();
    info!("End Basic fault library example");
}

fn playground() {
    let config = ReporterConfig {
        source: common::ids::SourceId {
            entity: to_static_short_string("source").unwrap(),
            ecu: Some(common::types::ShortString::from_bytes("ECU-A".as_bytes()).unwrap()),
            domain: Some(to_static_short_string("ADAS").unwrap()),
            sw_component: Some(to_static_short_string("Perception").unwrap()),
            instance: Some(to_static_short_string("0").unwrap()),
        },
        lifecycle_phase: LifecyclePhase::Running,
        default_env_data: MetadataVec::new(),
    };

    let sovd_path = FaultApi::get_fault_catalog().id.to_string();
    let mut faults = Vec::new();

    for desc in FaultApi::get_fault_catalog().descriptors() {
        faults.push(desc.id.clone());
    }

    let mut reporters = Vec::new();

    for fault in faults {
        reporters.push(Reporter::new(&fault, config.clone()).expect("get_descriptor failed"));
    }

    for x in 0..20 {
        debug!("Loop {x}");

        for reporter in reporters.iter_mut() {
            let stage = if (x % 2) == 0 {
                LifecycleStage::Passed
            } else {
                LifecycleStage::Failed
            };
            reporter
                .publish(&sovd_path, reporter.create_record(stage))
                .expect("publish failed");
        }
        thread::sleep(Duration::from_millis(200));
    }
}
