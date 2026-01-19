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
use clap::Parser;
use common::fault::*;
use env_logger::Env;
use fault_lib::FaultApi;
use fault_lib::catalog::FaultCatalogBuilder;

use fault_lib::reporter::Reporter;
use fault_lib::reporter::ReporterApi;
use fault_lib::test_utils::*;

use log::*;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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

    let env = Env::default().filter_or("RUST_LOG", "debug");
    env_logger::init_from_env(env);
    info!("Start Basic fault library example");
    // Create the FaultLib API object. We have to create it before any Fault API can be used
    // and keep it on stack until end of the program. No need to hand it over somewhere
    let _api = FaultApi::new(FaultCatalogBuilder::new().json_file(args.config_file).build());

    // here you can use any public api from fault-api
    playground();
    info!("End Basic fault library example");
}

fn playground() {
    let sovd_path = FaultApi::get_fault_catalog().id.to_string();
    let mut faults = Vec::new();

    for desc in FaultApi::get_fault_catalog().descriptors() {
        faults.push(desc.id.clone());
    }

    let mut reporters = Vec::new();

    for fault in faults {
        reporters.push(Reporter::new(&fault, stub_config()).expect("get_descriptor failed"));
    }

    for x in 0..20 {
        debug!("Loop {x}");

        for reporter in reporters.iter_mut() {
            let stage = if (x % 2) == 0 { LifecycleStage::Passed } else { LifecycleStage::Failed };
            reporter.publish(&sovd_path, reporter.create_record(stage)).expect("publish failed");
        }
        thread::sleep(Duration::from_millis(200));
    }
}
