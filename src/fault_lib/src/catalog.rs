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

// Re-export catalog types from common crate.
// FaultCatalog now lives in common so that dfm_lib can use it
// without depending on fault_lib.
pub use common::catalog::*;

#[cfg(test)]
#[cfg(not(miri))]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use common::{
        debounce::DebounceMode,
        fault::*,
        types::{to_static_long_string, to_static_short_string},
    };
    use iceoryx2_bb_container::vector::Vector;

    use super::*;

    /// Resolves test data file paths for both Cargo and Bazel test environments.
    /// Cargo runs from crate root, Bazel uses `CARGO_MANIFEST_DIR` env var.
    fn test_data_path(relative_path: &str) -> PathBuf {
        // First try CARGO_MANIFEST_DIR (works for both Cargo and Bazel with env set)
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let path = PathBuf::from(&manifest_dir).join(relative_path);
            if path.exists() {
                return path;
            }
        }
        // Fallback: relative path (Cargo default behavior)
        PathBuf::from(relative_path)
    }

    /// Test helper function - creates the test fault catalog configuration structure
    ///
    /// # Attention Any change in this function shall also be reflected in the `./tests/ivi_fault_catalog.json` file
    ///
    /// # Returns
    ///
    /// - `FaultCatalogConfig` - fault catalog test configuration.
    ///
    fn create_config() -> FaultCatalogConfig {
        FaultCatalogConfig {
            id: "ivi".into(),
            version: 1,
            faults: create_descriptors(),
        }
    }

    /// Creates test fault descriptors
    ///
    /// # Attention when you change something in the returned descriptors, please edit also
    /// `../tests/ivi_fault_catalog.json` file adequately
    ///
    /// # Returns
    ///
    /// - `Vec<FaultDescriptor>` - vector of test fault descriptors
    fn create_descriptors() -> Vec<FaultDescriptor> {
        use common::fault::*;

        let mut d1_compliance = ComplianceVec::new();
        let _ = d1_compliance.push(ComplianceTag::EmissionRelevant);
        let _ = d1_compliance.push(ComplianceTag::SafetyCritical);

        let mut d2_compliance = ComplianceVec::new();
        let _ = d2_compliance.push(ComplianceTag::SecurityRelevant);
        let _ = d2_compliance.push(ComplianceTag::SafetyCritical);

        vec![
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("d1").unwrap()),

                name: to_static_short_string("Descriptor 1").unwrap(),
                summary: None,

                category: FaultType::Software,
                severity: FaultSeverity::Debug,
                compliance: ComplianceVec::try_from(
                    &[
                        ComplianceTag::EmissionRelevant,
                        ComplianceTag::SafetyCritical,
                    ][..],
                )
                .unwrap(),

                reporter_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(100u64).into(),
                }),
                reporter_side_reset: None,
                manager_side_debounce: None,
                manager_side_reset: None,
            },
            FaultDescriptor {
                id: FaultId::Text(to_static_short_string("d2").unwrap()),

                name: to_static_short_string("Descriptor 2").unwrap(),
                summary: Some(to_static_long_string("Human-readable summary").unwrap()),

                category: FaultType::Configuration,
                severity: FaultSeverity::Warn,
                compliance: ComplianceVec::try_from(
                    &[
                        ComplianceTag::SecurityRelevant,
                        ComplianceTag::SafetyCritical,
                    ][..],
                )
                .unwrap(),

                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(100u64).into(),
                }),
                manager_side_reset: None,
            },
        ]
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn from_config() {
        let cfg = create_config();

        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(cfg.clone())
            .unwrap()
            .build();

        let d1 = catalog
            .descriptor(&FaultId::Text(to_static_short_string("d1").unwrap()))
            .expect("get_descriptor failed");
        let d2 = catalog
            .descriptor(&FaultId::Text(to_static_short_string("d2").unwrap()))
            .expect("get_descriptor failed");

        assert_eq!(*d1, cfg.faults[0]);
        assert_eq!(*d2, cfg.faults[1]);
    }

    #[test]
    fn empty_config() {
        let cfg = FaultCatalogConfig {
            id: "".into(),
            version: 7,
            faults: Vec::new(),
        };

        let catalog = FaultCatalogBuilder::new()
            .cfg_struct(cfg.clone())
            .unwrap()
            .build();
        let d1 = catalog.descriptor(&FaultId::Text(to_static_short_string("d1").unwrap()));
        assert_eq!(d1, Option::None);
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn from_json_string() {
        let cfg = create_config();
        let json_string = serde_json::to_string_pretty(&cfg).unwrap();

        let fault_catalog = FaultCatalogBuilder::new()
            .json_string(json_string.as_str())
            .unwrap()
            .build();
        let d1 = fault_catalog
            .descriptor(&FaultId::Text(to_static_short_string("d1").unwrap()))
            .expect("get_descriptor failed");
        let d2 = fault_catalog
            .descriptor(&FaultId::Text(to_static_short_string("d2").unwrap()))
            .expect("get_descriptor failed");

        assert_eq!(*d1, cfg.faults[0]);
        assert_eq!(*d2, cfg.faults[1]);
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn from_json_file() {
        // Note: use test_data_path helper for Cargo/Bazel compatibility
        let fault_catalog = FaultCatalogBuilder::new()
            .json_file(test_data_path("tests/data/ivi_fault_catalog.json"))
            .unwrap()
            .build();
        let d1 = fault_catalog
            .descriptor(&FaultId::Text(to_static_short_string("d1").unwrap()))
            .expect("get_descriptor failed");
        let d2 = fault_catalog
            .descriptor(&FaultId::Text(to_static_short_string("d2").unwrap()))
            .expect("get_descriptor failed");
        // create a reference catalog config - shall be equal to the one in json
        let cfg = create_config();

        assert_eq!(*d1, cfg.faults[0]);
        assert_eq!(*d2, cfg.faults[1]);
    }

    #[test]
    #[should_panic(expected = "Failed to build FaultCatalog")]
    fn from_not_existing_json_file() {
        let _ = FaultCatalogBuilder::new()
            .json_file(PathBuf::from("tests/data/xxx.json"))
            .unwrap()
            .build();
    }

    #[test]
    fn hash_sum() {
        let catalog_from_file = FaultCatalogBuilder::new()
            .json_file(test_data_path("tests/data/ivi_fault_catalog.json"))
            .unwrap()
            .build();
        let cfg = create_config();
        let catalog_from_cfg = FaultCatalogBuilder::new()
            .cfg_struct(cfg.clone())
            .unwrap()
            .build();
        let catalog_from_json = FaultCatalogBuilder::new()
            .json_string(&serde_json::to_string_pretty(&cfg).unwrap())
            .unwrap()
            .build();

        assert_eq!(
            catalog_from_cfg.config_hash(),
            catalog_from_file.config_hash()
        );
        assert_eq!(
            catalog_from_cfg.config_hash(),
            catalog_from_json.config_hash()
        );
    }
}
