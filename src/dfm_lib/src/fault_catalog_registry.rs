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

//! In-memory registry of fault catalogs received during handshake.
//!
//! Each connected application registers its [`FaultCatalog`] keyed by an
//! entity path.  The registry is consulted by
//! [`FaultRecordProcessor`](crate::fault_record_processor::FaultRecordProcessor)
//! for hash verification and debounce lookup, and by
//! [`SovdFaultManager`](crate::sovd_fault_manager::SovdFaultManager) for
//! descriptor resolution.

use alloc::borrow::Cow;
use common::catalog::FaultCatalog;
use std::collections::HashMap;

/// In-memory registry of fault catalogs, keyed by their entity path.
///
/// Each connected application registers its catalog during handshake.
/// The registry is used by `SovdFaultManager` to resolve fault descriptors
/// and by `FaultRecordProcessor` for hash verification and debounce lookup.
pub struct FaultCatalogRegistry {
    pub(crate) catalogs: HashMap<Cow<'static, str>, FaultCatalog>,
}

impl FaultCatalogRegistry {
    pub fn new(entries: Vec<FaultCatalog>) -> Self {
        let mut catalogs = HashMap::with_capacity(entries.len());
        for entry in entries {
            if catalogs.contains_key(&entry.id) {
                tracing::warn!(
                    "Duplicate catalog ID '{}' - overwriting previous entry",
                    entry.id
                );
            }
            catalogs.insert(entry.id.clone(), entry);
        }
        Self { catalogs }
    }

    #[must_use]
    pub fn get(&self, path: &str) -> Option<&FaultCatalog> {
        self.catalogs.get(path)
    }
}
