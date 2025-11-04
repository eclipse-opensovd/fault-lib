/*
* Copyright (c) 2025 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
*
* See the NOTICE file(s) distributed with this work for additional
* information regarding copyright ownership.
*
* This program and the accompanying materials are made available under the
* terms of the Apache License Version 2.0 which is available at
* https://www.apache.org/licenses/LICENSE-2.0
*
* SPDX-License-Identifier: Apache-2.0
*/

use crate::{ids::FaultId, model::FaultDescriptor};
use std::borrow::Cow;

/// Declarative catalog shared between reporters and the Diagnostic Fault Manager.
#[derive(Clone, Debug)]
pub struct FaultCatalog {
    pub id: Cow<'static, str>,
    pub version: u64,
    pub descriptors: Cow<'static, [FaultDescriptor]>,
}

impl FaultCatalog {
    pub const fn new(
        id: &'static str,
        version: u64,
        descriptors: &'static [FaultDescriptor],
    ) -> Self {
        Self {
            id: Cow::Borrowed(id),
            version,
            descriptors: Cow::Borrowed(descriptors),
        }
    }

    /// When the DFM deserializes a JSON/YAML catalog at startup, this helper
    /// lets it hand the owned data back to the library without rebuilding.
    pub fn from_config(
        id: impl Into<Cow<'static, str>>,
        version: u64,
        descriptors: Vec<FaultDescriptor>,
    ) -> Self {
        Self {
            id: id.into(),
            version,
            descriptors: Cow::Owned(descriptors),
        }
    }

    /// Locate a descriptor by its FaultId, handy for tests or build tooling.
    pub fn find(&self, id: &FaultId) -> Option<&FaultDescriptor> {
        self.descriptors.iter().find(|d| &d.id == id)
    }

    /// Number of descriptors in this catalog, useful for build-time validation.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }
}
