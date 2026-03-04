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

use crate::fault::{FaultDescriptor, FaultId};
use crate::types::LongString;
use alloc::borrow::Cow;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::PathBuf};

/// Error type for fault catalog building failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogBuildError {
    /// The input string was not valid JSON.
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    /// No configuration source was set before calling `try_build`.
    #[error("missing configuration")]
    MissingConfig,

    /// An I/O error occurred while reading a JSON file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The catalog identifier exceeds the 128-byte IPC limit.
    #[error("catalog id too long for IPC: {0}")]
    IdTooLong(String),

    /// A source was already configured on this builder.
    #[error("builder already configured")]
    AlreadyConfigured,

    /// Two descriptors share the same [`FaultId`].
    #[error("duplicate FaultId: {0:?}")]
    DuplicateFaultId(FaultId),
}

type FaultDescriptorsMap = HashMap<FaultId, FaultDescriptor>;
type FaultCatalogHash = Vec<u8>;

/// Runtime representation of a fault catalog.
///
/// A `FaultCatalog` bundles a set of [`FaultDescriptor`]s under a shared
/// identifier and version, together with a SHA-256 hash of the serialised
/// configuration.  The hash is used during the DFM handshake to verify
/// that reporter and manager agree on the same catalog revision.
#[derive(Debug)]
pub struct FaultCatalog {
    /// Human-readable catalog identifier (e.g. `"hvac"`).
    pub id: Cow<'static, str>,
    /// Monotonically increasing catalog revision number.
    pub version: u64,
    descriptors: FaultDescriptorsMap,
    config_hash: FaultCatalogHash,
}

impl FaultCatalog {
    /// Create a new `FaultCatalog` from pre-built components.
    ///
    /// Prefer [`FaultCatalogBuilder`] for constructing catalogs from JSON
    /// or [`FaultCatalogConfig`] structs — it handles hashing and
    /// duplicate-ID detection automatically.
    #[must_use]
    pub fn new(
        id: Cow<'static, str>,
        version: u64,
        descriptors: FaultDescriptorsMap,
        config_hash: FaultCatalogHash,
    ) -> Self {
        Self {
            id,
            version,
            descriptors,
            config_hash,
        }
    }

    /// SHA-256 hash of the canonical JSON representation of this catalog's
    /// configuration.  Used during the DFM handshake to detect version drift
    /// between reporter and manager.
    #[must_use]
    pub fn config_hash(&self) -> &[u8] {
        &self.config_hash
    }

    /// Try to get the catalog id as a fixed-size IPC-safe `LongString`.
    ///
    /// # Errors
    ///
    /// Returns `CatalogBuildError::IdTooLong` if the id exceeds 128 bytes.
    pub fn try_id(&self) -> Result<LongString, CatalogBuildError> {
        LongString::try_from(self.id.as_bytes())
            .map_err(|_| CatalogBuildError::IdTooLong(self.id.to_string()))
    }

    /// Get the catalog id as a `LongString`.
    ///
    /// # Panics
    ///
    /// Panics if the id exceeds 128 bytes. Use [`try_id`](Self::try_id)
    /// for fallible access.
    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn id(&self) -> LongString {
        self.try_id().expect("Fault catalog id too long")
    }

    /// Look up a single descriptor by its [`FaultId`].
    ///
    /// Returns `None` if the catalog does not contain a descriptor with the
    /// given ID.
    #[must_use]
    pub fn descriptor(&self, id: &FaultId) -> Option<&FaultDescriptor> {
        self.descriptors.get(id)
    }

    /// Return an iterator over all descriptors in this catalog.
    ///
    /// Iteration order is unspecified (backed by `HashMap`).
    pub fn descriptors(&self) -> impl Iterator<Item = &FaultDescriptor> {
        self.descriptors.values()
    }

    /// Number of descriptors in this catalog, useful for build-time validation.
    #[must_use]
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// Returns `true` if this catalog contains no descriptors.
    #[must_use]
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
    /// Unique identifier for this fault catalog (e.g. `"hvac"`).
    pub id: Cow<'static, str>,
    /// Monotonically increasing catalog revision number.
    pub version: u64,
    /// Ordered list of fault descriptors belonging to this catalog.
    pub faults: Vec<FaultDescriptor>,
}
/// Input source for [`FaultCatalogBuilder`].
///
/// Exactly one source must be set before calling
/// [`FaultCatalogBuilder::try_build`].
#[non_exhaustive]
pub enum FaultCatalogBuilderInput<'a> {
    /// No source configured yet.
    None,
    /// Raw JSON string.
    JsonString(&'a str),
    /// Path to a JSON file on disk.
    JsonFile(PathBuf),
    /// Pre-built configuration struct.
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
/// - `Self` - `FaultCatalogBuilder` structure.
///
impl Default for FaultCatalogBuilder<'_> {
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
    ///   * `FaultCatalogBuilder` instance
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if the builder has been not configured yet.
    ///
    /// # Errors
    ///
    /// Returns `CatalogBuildError::AlreadyConfigured` if a source was already set.
    fn check_if_not_set(&self) -> Result<(), CatalogBuildError> {
        if !matches!(self.input, FaultCatalogBuilderInput::None) {
            return Err(CatalogBuildError::AlreadyConfigured);
        }
        Ok(())
    }

    /// Configure `FaultCatalog` with given json configuration string.
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
    /// # Errors
    ///
    /// Returns `CatalogBuildError::AlreadyConfigured` if a source was already set.
    pub fn json_string(mut self, json_string: &'a str) -> Result<Self, CatalogBuildError> {
        self.check_if_not_set()?;
        self.input = FaultCatalogBuilderInput::JsonString(json_string);
        Ok(self)
    }

    /// Configure the `FaultCatalog` with the given JSON configuration file.
    ///
    /// Only one source may be set per builder — calling this after another
    /// source method returns an error.
    ///
    /// # Arguments
    ///
    /// * `json_file` — path to the `FaultCatalog` JSON configuration file.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogBuildError::AlreadyConfigured`] if a source was already set.
    pub fn json_file(mut self, json_file: PathBuf) -> Result<Self, CatalogBuildError> {
        self.check_if_not_set()?;
        self.input = FaultCatalogBuilderInput::JsonFile(json_file);
        Ok(self)
    }

    /// Configure the `FaultCatalog` from a pre-built [`FaultCatalogConfig`].
    ///
    /// Only one source may be set per builder — calling this after another
    /// source method returns an error.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogBuildError::AlreadyConfigured`] if a source was already set.
    pub fn cfg_struct(mut self, cfg: FaultCatalogConfig) -> Result<Self, CatalogBuildError> {
        self.check_if_not_set()?;
        self.input = FaultCatalogBuilderInput::ConfigStruct(cfg);
        Ok(self)
    }

    /// Builds the `FaultCatalog`.
    ///
    /// # Errors
    ///
    /// Returns `CatalogBuildError` if configuration is missing, JSON is invalid,
    /// or the file cannot be read.
    pub fn try_build(self) -> Result<FaultCatalog, CatalogBuildError> {
        match self.input {
            FaultCatalogBuilderInput::JsonString(json_str) => Self::try_from_json_string(json_str),
            FaultCatalogBuilderInput::JsonFile(json_file) => Self::try_from_file(json_file),
            FaultCatalogBuilderInput::ConfigStruct(cfg_struct) => Self::from_cfg_struct(cfg_struct),
            FaultCatalogBuilderInput::None => Err(CatalogBuildError::MissingConfig),
        }
    }

    /// Builds the `FaultCatalog`, panicking on error.
    ///
    /// # Panics
    ///
    /// Panics if configuration is missing, JSON is invalid, or the file cannot be read.
    /// Use [`try_build`](Self::try_build) for the fallible version.
    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn build(self) -> FaultCatalog {
        self.try_build().expect("Failed to build FaultCatalog")
    }

    /// Build a [`FaultCatalog`] from a [`FaultCatalogConfig`].
    ///
    /// Computes the SHA-256 config hash, converts the descriptor list into an
    /// indexed map, and validates that no duplicate [`FaultId`]s exist.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogBuildError::DuplicateFaultId`] if two descriptors
    /// share the same ID, or a serialisation error if hashing fails.
    fn from_cfg_struct(cfg_struct: FaultCatalogConfig) -> Result<FaultCatalog, CatalogBuildError> {
        let hash_sum = Self::calc_config_hash(&cfg_struct)?;
        let mut descriptors = FaultDescriptorsMap::new();
        for descriptor in cfg_struct.faults {
            let id = descriptor.id.clone();
            if descriptors.contains_key(&id) {
                return Err(CatalogBuildError::DuplicateFaultId(id));
            }
            descriptors.insert(id, descriptor);
        }
        Ok(FaultCatalog::new(
            cfg_struct.id,
            cfg_struct.version,
            descriptors,
            hash_sum,
        ))
    }

    /// Fallible version: generates fault catalog from JSON string.
    fn try_from_json_string(json: &str) -> Result<FaultCatalog, CatalogBuildError> {
        let cfg: FaultCatalogConfig = serde_json::from_str(json)?;
        Self::from_cfg_struct(cfg)
    }

    /// Fallible version: creates fault catalog from a JSON file.
    fn try_from_file(json_path: PathBuf) -> Result<FaultCatalog, CatalogBuildError> {
        let cfg_file_txt = fs::read_to_string(json_path)?;
        Self::try_from_json_string(&cfg_file_txt)
    }

    /// Compute the SHA-256 hash of the canonical JSON serialisation of `cfg`.
    ///
    /// The canonical form is produced by `serde_json::to_string`, ensuring
    /// that two structurally identical configs always yield the same hash.
    ///
    /// # Errors
    ///
    /// Returns a [`CatalogBuildError`] if JSON serialisation fails.
    fn calc_config_hash(cfg: &FaultCatalogConfig) -> Result<Vec<u8>, CatalogBuildError> {
        let canon = serde_json::to_string(cfg)?;
        Ok(Sha256::new()
            .chain_update(canon.as_bytes())
            .finalize()
            .to_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use crate::fault::{ComplianceVec, FaultSeverity, FaultType};
    use crate::types::to_static_short_string;

    fn make_descriptor(id: FaultId) -> FaultDescriptor {
        FaultDescriptor {
            id,
            name: to_static_short_string("Test").unwrap(),
            summary: None,
            category: FaultType::Software,
            severity: FaultSeverity::Warn,
            compliance: ComplianceVec::new(),
            reporter_side_debounce: None,
            reporter_side_reset: None,
            manager_side_debounce: None,
            manager_side_reset: None,
        }
    }

    #[test]
    fn duplicate_fault_id_returns_error() {
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![
                make_descriptor(FaultId::Numeric(42)),
                make_descriptor(FaultId::Numeric(42)),
            ],
        };
        let result = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .try_build();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CatalogBuildError::DuplicateFaultId(_)
        ));
    }

    #[test]
    fn unique_fault_ids_build_successfully() {
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![
                make_descriptor(FaultId::Numeric(1)),
                make_descriptor(FaultId::Numeric(2)),
                make_descriptor(FaultId::Text(to_static_short_string("fault_a").unwrap())),
            ],
        };
        let result = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .try_build();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn numeric_and_text_with_same_value_are_not_duplicates() {
        let config = FaultCatalogConfig {
            id: "test".into(),
            version: 1,
            faults: vec![
                make_descriptor(FaultId::Numeric(1)),
                make_descriptor(FaultId::Text(to_static_short_string("1").unwrap())),
            ],
        };
        let result = FaultCatalogBuilder::new()
            .cfg_struct(config)
            .unwrap()
            .try_build();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }
}
