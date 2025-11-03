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

/// Declarative catalog shared between reporters and the Diagnostic Fault Manager.
#[derive(Clone, Debug)]
pub struct FaultCatalog {
    pub id: &'static str,
    pub version: u64,
    pub descriptors: &'static [FaultDescriptor],
}

impl FaultCatalog {
    pub const fn new(
        id: &'static str,
        version: u64,
        descriptors: &'static [FaultDescriptor],
    ) -> Self {
        Self {
            id,
            version,
            descriptors,
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
