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

use common::fault::*;
use common::types::LongString;
use log::error;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::panic;
use std::{collections::HashMap, fs, path::PathBuf};

type FaultDescriptorsMap = HashMap<FaultId, FaultDescriptor>;
type FaultCatalogHash = Vec<u8>;
pub struct FaultCatalog {
    pub id: Cow<'static, str>,
    pub version: u64,
    descriptors: FaultDescriptorsMap,
    config_hash: FaultCatalogHash,
}

impl FaultCatalog {
    pub(crate) fn new(id: Cow<'static, str>, version: u64, descriptors: FaultDescriptorsMap, config_hash: FaultCatalogHash) -> Option<Self> {
        Some(Self {
            id,
            version,
            descriptors,
            config_hash,
        })
    }

    #[allow(unused)]
    pub fn config_hash(&self) -> &[u8] {
        &self.config_hash
    }

    pub fn id(&self) -> LongString {
        LongString::from_str_truncated(&self.id).expect("Fault catalog id too long")
    }

    pub fn descriptor(&self, id: &FaultId) -> Option<&FaultDescriptor> {
        self.descriptors.get(id)
    }

    pub fn descriptors(&self) -> Vec<&FaultDescriptor> {
        self.descriptors.values().collect()
    }

    /// Number of descriptors in this catalog, useful for build-time validation.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }
}

/// Fault Catalog configuration structure
///
/// Can be used for code generation of fault catalog configuration.
///
/// # Fields
///
/// - `id` (`Cow<'static`) - fault catalog ID .
/// - `version` (`u64`) - the version of the fault catalog.
/// - `faults` (`Vec<FaultDescriptor>`) - vector of fault descriptors.
///
#[derive(Debug, serde::Deserialize, serde::Serialize, Clone)]
pub struct FaultCatalogConfig {
    pub id: Cow<'static, str>,
    pub version: u64,
    pub faults: Vec<FaultDescriptor>,
}
pub enum FaultCatalogBuilderInput<'a> {
    None,
    JsonString(&'a str),
    JsonFile(PathBuf),
    ConfigStruct(FaultCatalogConfig),
}

/// Fault Catalog builder
pub struct FaultCatalogBuilder<'a> {
    input: FaultCatalogBuilderInput<'a>,
}

/// Implementation of the Default trait for the fault catalog builder
///
/// # Returns
///
/// - `Self` - FaultCatalogBuilder structure.
///
impl<'a> Default for FaultCatalogBuilder<'a> {
    fn default() -> Self {
        Self {
            input: FaultCatalogBuilderInput::None,
        }
    }
}

impl<'a> FaultCatalogBuilder<'a> {
    /// Fault catalog builder constructor
    ///
    /// # Return Values
    ///   * FaultCatalogBuilder instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if the builder has been not configured yet
    ///
    /// # Arguments
    ///
    /// - `&self` - FaultCatalogBuilder
    fn check_if_not_set(&self) {
        if !matches!(self.input, FaultCatalogBuilderInput::None) {
            panic!("Fault Catalog builder already configured");
        }
    }

    /// Configure 'FaultCatalog' with given json configuration string.
    ///
    ///  You cannot use this function in case the configuration file has been passed before
    /// # Arguments
    ///
    /// - `mut self` - the builder itself.
    /// - `json_string` (`&'a str`) - the fault catalog configuration string in json format
    ///
    /// # Returns
    ///
    /// - `Self` - the `FaultCatalogBuilder` instance.
    pub fn json_string(mut self, json_string: &'a str) -> Self {
        self.check_if_not_set();
        self.input = FaultCatalogBuilderInput::JsonString(json_string);
        self
    }

    /// Configure the `FaultCatalog` with the given json configuration file.
    ///
    /// You cannot use this function in case the configuration string or fault descriptors have been
    /// passed before
    ///
    /// # Arguments
    ///
    /// - `json_file` (`PathBuf`) - tha path to the `FaultCatalog` configuration file.
    ///
    /// # Returns
    ///
    /// - `Self` - The `FaultCatalogBuilder` instance .
    pub fn json_file(mut self, json_file: PathBuf) -> Self {
        self.check_if_not_set();
        self.input = FaultCatalogBuilderInput::JsonFile(json_file);
        self
    }

    pub fn cfg_struct(mut self, cfg: FaultCatalogConfig) -> Self {
        self.check_if_not_set();
        self.input = FaultCatalogBuilderInput::ConfigStruct(cfg);
        self
    }

    /// Builds the `FaultCatalog`
    ///
    /// The build operation will panic in case the configuration file cannot be open
    /// or the configuration json format is invalid
    ///
    /// # Returns
    ///
    /// - `FaultCatalog` - the fault catalog instance .
    ///
    pub fn build(self) -> FaultCatalog {
        match self.input {
            FaultCatalogBuilderInput::JsonString(json_str) => Self::from_json_string(json_str),
            FaultCatalogBuilderInput::JsonFile(json_file) => Self::from_file(json_file),
            FaultCatalogBuilderInput::ConfigStruct(cfg_struct) => Self::from_cfg_struct(cfg_struct),

            FaultCatalogBuilderInput::None => panic!("Missing fault catalog configuration"),
        }
    }

    /// Help function which creates `FaultCatalog` object from configuration structure
    /// and calculates the `FaultCatalog` hash sum
    ///
    /// # Arguments
    ///
    /// - `cfg_struct` (`FaultCatalogConfig`) - Describe this parameter.
    ///
    /// # Returns
    ///
    /// - `FaultCatalog` - Describe the return value.
    fn from_cfg_struct(cfg_struct: FaultCatalogConfig) -> FaultCatalog {
        let hash_sum = Self::calc_config_hash(&cfg_struct);
        FaultCatalog::new(
            cfg_struct.id,
            cfg_struct.version,
            cfg_struct
                .faults
                .into_iter()
                .map(|descriptor| (descriptor.id.clone(), descriptor))
                .collect(),
            hash_sum,
        )
        .expect("Cannot create FaultCatalog from config")
    }

    /// Help function which generates fault catalog object from the the configuration json string
    ///
    ///
    /// # Arguments
    ///
    /// - `json` (`&str`) - fault catalog configuration string.
    ///
    /// # Returns
    ///
    /// - `FaultCatalog` - fault catalog structure.
    fn from_json_string(json: &str) -> FaultCatalog {
        println!("Json: {:?}", json);
        let cfg = Self::deserialize_config(json);
        Self::from_cfg_struct(cfg)
    }

    /// Creates the fault catalog from the given json configuration file.
    ///
    /// # Arguments
    ///
    /// - `json_path` (`PathBuf`) - path to the fault catalog json configuration file.
    ///
    /// # Returns
    ///
    /// - `FaultCatalog` - fault catalog structure.
    ///
    fn from_file(json_path: PathBuf) -> FaultCatalog {
        let cfg_file_txt = fs::read_to_string(json_path).expect("Cannot read the json fault catalog file");
        Self::from_json_string(&cfg_file_txt)
    }

    /// Calculates hash sum for the fault catalog json string
    ///
    /// # Arguments
    ///
    /// - `cfg` (`&str`) - fault catalog configuration string.
    ///
    /// # Returns
    ///
    /// - `Vec<u8>` - hash sum for the fault catalog.
    ///
    fn calc_config_hash(cfg: &FaultCatalogConfig) -> Vec<u8> {
        let canon = serde_json::to_string(cfg).expect("Failed to serialize FaultCatalogConfig for hashing");
        Sha256::new().chain_update(canon.as_bytes()).finalize().to_vec()
    }

    /// Deserialize json configuration string to the `FaultCatalogConfig` structure
    ///
    ///
    ///
    /// # Arguments
    ///
    /// - `config` (`&str`) - the fault catalog configuration json string .
    ///
    /// # Returns
    ///
    /// - `FaultCatalogConfig` - fault catalog configuration structure.
    ///
    fn deserialize_config(config: &str) -> FaultCatalogConfig {
        match serde_json::from_str::<FaultCatalogConfig>(config) {
            Ok(cfg) => cfg,
            Err(e) => {
                error!("Failed to deserialize config: {}", e);
                panic!("Invalid fault catalog json");
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(miri))]
mod tests {
    use super::*;
    use crate::utils::*;
    use common::debounce::DebounceMode;
    use iceoryx2_bb_container::vector::Vector;
    use std::time::Duration;

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
                compliance: ComplianceVec::try_from(&[ComplianceTag::EmissionRelevant, ComplianceTag::SafetyCritical][..]).unwrap(),

                reporter_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(100_u64),
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
                compliance: ComplianceVec::try_from(&[ComplianceTag::SecurityRelevant, ComplianceTag::SafetyCritical][..]).unwrap(),

                reporter_side_debounce: None,
                reporter_side_reset: None,
                manager_side_debounce: Some(DebounceMode::EdgeWithCooldown {
                    cooldown: Duration::from_millis(100_u64),
                }),
                manager_side_reset: None,
            },
        ]
    }

    #[test]
    fn from_config() {
        let cfg = create_config();

        let catalog = FaultCatalogBuilder::new().cfg_struct(cfg.clone()).build();

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

        let catalog = FaultCatalogBuilder::new().cfg_struct(cfg.clone()).build();
        let d1 = catalog.descriptor(&FaultId::Text(to_static_short_string("d1").unwrap()));
        assert_eq!(d1, Option::None);
    }

    #[test]
    fn from_json_string() {
        let cfg = create_config();
        let json_string = serde_json::to_string_pretty(&cfg).unwrap();

        let fault_catalog = FaultCatalogBuilder::new().json_string(json_string.as_str()).build();
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
    fn from_json_file() {
        // Note: the path here is relative to the fault_lib directory
        let fault_catalog = FaultCatalogBuilder::new()
            .json_file(PathBuf::from("tests/data/ivi_fault_catalog.json"))
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
    #[should_panic]
    fn from_not_existing_json_file() {
        let _ = FaultCatalogBuilder::new().json_file(PathBuf::from("tests/data/xxx.json")).build();
    }

    #[test]
    fn hash_sum() {
        let catalog_from_file = FaultCatalogBuilder::new()
            .json_file(PathBuf::from("tests/data/ivi_fault_catalog.json"))
            .build();
        let cfg = create_config();
        let catalog_from_cfg = FaultCatalogBuilder::new().cfg_struct(cfg.clone()).build();
        let catalog_from_json = FaultCatalogBuilder::new()
            .json_string(&serde_json::to_string_pretty(&cfg).unwrap())
            .build();

        assert_eq!(catalog_from_cfg.config_hash(), catalog_from_file.config_hash());
        assert_eq!(catalog_from_cfg.config_hash(), catalog_from_json.config_hash());
    }
}
