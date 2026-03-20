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

use std::path::PathBuf;

use clap::Parser;
use common::catalog::FaultCatalogBuilder;
use dfm_lib::{
    diagnostic_fault_manager::DiagnosticFaultManager, fault_catalog_registry::FaultCatalogRegistry,
    sovd_fault_storage::KvsSovdFaultStateStorage,
};
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "dfm", about = "Diagnostic Fault Manager - standalone binary")]
struct Cli {
    /// Directory containing JSON fault catalog files (*.json)
    #[arg(long)]
    catalog_dir: PathBuf,

    /// Directory for KVS persistent storage
    #[arg(long)]
    storage_dir: PathBuf,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    if !cli.catalog_dir.is_dir() {
        error!(
            "Catalog directory does not exist: {}",
            cli.catalog_dir.display()
        );
        std::process::exit(1);
    }

    if !cli.storage_dir.exists() {
        info!("Creating storage directory: {}", cli.storage_dir.display());
        if let Err(e) = std::fs::create_dir_all(&cli.storage_dir) {
            error!("Failed to create storage directory: {e}");
            std::process::exit(1);
        }
    }

    let json_files: Vec<PathBuf> = match std::fs::read_dir(&cli.catalog_dir) {
        Ok(entries) => entries
            .filter_map(core::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect(),
        Err(e) => {
            error!("Failed to read catalog directory: {e}");
            std::process::exit(1);
        }
    };

    if json_files.is_empty() {
        error!(
            "No JSON catalog files found in {}",
            cli.catalog_dir.display()
        );
        std::process::exit(1);
    }

    let mut catalogs = Vec::new();
    for path in &json_files {
        let json = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to read {}: {e}", path.display());
                std::process::exit(1);
            }
        };

        let catalog = match FaultCatalogBuilder::new().json_string(&json) {
            Ok(builder) => builder.build(),
            Err(e) => {
                error!("Failed to parse catalog {}: {e:?}", path.display());
                std::process::exit(1);
            }
        };

        info!(
            "Loaded catalog '{}' from {} ({} faults)",
            catalog.id,
            path.display(),
            catalog.len()
        );
        catalogs.push(catalog);
    }

    let total_faults: usize = catalogs
        .iter()
        .map(common::catalog::FaultCatalog::len)
        .sum();
    let registry = FaultCatalogRegistry::new(catalogs);

    let storage = match KvsSovdFaultStateStorage::new(&cli.storage_dir, 0) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initialize KVS storage: {e:?}");
            std::process::exit(1);
        }
    };

    info!(
        "Starting DFM with query server ({total_faults} faults across {} catalogs)",
        json_files.len()
    );

    // DiagnosticFaultManager::with_query_server() spawns a receiver thread
    // internally. The DFM runs until this value is dropped.
    let _dfm = DiagnosticFaultManager::with_query_server(storage, registry);

    info!("DFM ready");

    // Park main thread until SIGINT/SIGTERM.
    loop {
        std::thread::park();
    }
}
