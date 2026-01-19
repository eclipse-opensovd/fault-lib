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

use fault_lib::catalog::FaultCatalog;
use std::{borrow::Cow, collections::HashMap};

//TODO: The registry was needed to implement the SovdFaultManager. This is just a mock, and will be changed.

pub struct FaultCatalogRegistry {
    pub(crate) catalogs: HashMap<Cow<'static, str>, FaultCatalog>,
}

impl FaultCatalogRegistry {
    pub fn new(entries: Vec<FaultCatalog>) -> Self {
        Self {
            catalogs: entries.into_iter().map(|kv| (kv.id.clone(), kv)).collect(),
        }
    }

    pub fn get(&self, path: &str) -> Option<&FaultCatalog> {
        self.catalogs.get(path)
    }
}
