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

//! Abstraction over DFM fault query/clear operations.
//!
//! [`DfmQueryApi`] enables both in-process ([`DirectDfmQuery`]) and remote
//! (IPC) access to the Diagnostic Fault Manager's SOVD-compliant fault store.

use crate::fault_catalog_registry::FaultCatalogRegistry;
use crate::sovd_fault_manager::{Error, SovdEnvData, SovdFault, SovdFaultManager};
use crate::sovd_fault_storage::SovdFaultStateStorage;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Abstraction over DFM fault query/clear operations.
///
/// Enables both in-process (glue code) and remote (IPC) access
/// to the Diagnostic Fault Manager's SOVD-compliant fault store.
///
/// Object-safe: can be used as `Box<dyn DfmQueryApi>` or `&dyn DfmQueryApi`.
/// # Errors
///
/// All methods return [`Error`] on bad arguments, not-found, or storage failures.
pub trait DfmQueryApi: Send + Sync {
    /// List all known faults for a given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the path is unknown or the storage backend fails.
    fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error>;

    /// Get a single fault with its environment data snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the path or fault code is unknown.
    fn get_fault(&self, path: &str, fault_code: &str) -> Result<(SovdFault, SovdEnvData), Error>;

    /// Clear all stored faults for a given entity path.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the path is unknown or the storage backend fails.
    fn delete_all_faults(&self, path: &str) -> Result<(), Error>;

    /// Clear a single stored fault by its code.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the path or fault code is unknown.
    fn delete_fault(&self, path: &str, fault_code: &str) -> Result<(), Error>;
}

/// In-process [`DfmQueryApi`] implementation - zero-cost delegation to
/// [`SovdFaultManager`].
///
/// Use this when the SOVD consumer runs in the same process as the DFM
/// (glue-code / embedded scenario).
pub struct DirectDfmQuery<S: SovdFaultStateStorage> {
    inner: SovdFaultManager<S>,
}

impl<S: SovdFaultStateStorage> DirectDfmQuery<S> {
    /// Wrap an existing [`SovdFaultManager`] as a [`DfmQueryApi`] implementor.
    pub fn new(storage: Arc<S>, registry: Arc<FaultCatalogRegistry>) -> Self {
        Self {
            inner: SovdFaultManager::new(storage, registry),
        }
    }
}

impl<S: SovdFaultStateStorage> DfmQueryApi for DirectDfmQuery<S> {
    fn get_all_faults(&self, path: &str) -> Result<Vec<SovdFault>, Error> {
        self.inner.get_all_faults(path)
    }

    fn get_fault(&self, path: &str, fault_code: &str) -> Result<(SovdFault, SovdEnvData), Error> {
        self.inner.get_fault(path, fault_code)
    }

    fn delete_all_faults(&self, path: &str) -> Result<(), Error> {
        self.inner.delete_all_faults(path)
    }

    fn delete_fault(&self, path: &str, fault_code: &str) -> Result<(), Error> {
        self.inner.delete_fault(path, fault_code)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc
)]
mod tests {
    use super::*;
    use crate::dfm_test_utils::*;
    use crate::fault_record_processor::FaultRecordProcessor;
    use common::fault::{FaultId, LifecycleStage};
    use common::types::to_static_short_string;
    use std::sync::Arc;

    fn make_direct_query(
        storage: Arc<InMemoryStorage>,
        registry: Arc<FaultCatalogRegistry>,
    ) -> DirectDfmQuery<InMemoryStorage> {
        DirectDfmQuery::new(storage, registry)
    }

    /// `DirectDfmQuery` delegates `get_all_faults` correctly.
    #[test]
    fn direct_query_get_all_faults() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            make_cycle_tracker(),
        );
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let query: &dyn DfmQueryApi = &make_direct_query(storage, registry);
        let faults = query.get_all_faults("test_entity").unwrap();
        assert_eq!(faults.len(), 2); // 2 descriptors in catalog
    }

    /// `DirectDfmQuery` delegates `get_fault` correctly.
    #[test]
    fn direct_query_get_fault() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            make_cycle_tracker(),
        );
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let query = make_direct_query(storage, registry);
        let (fault, _env) = query.get_fault("test_entity", "fault_a").unwrap();
        assert_eq!(fault.code, "fault_a");
        assert!(fault.typed_status.as_ref().unwrap().test_failed.unwrap());
    }

    /// `DirectDfmQuery` delegates `delete_fault` correctly.
    #[test]
    fn direct_query_delete_fault() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            make_cycle_tracker(),
        );
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let query = make_direct_query(storage, registry);
        assert!(query.delete_fault("test_entity", "fault_a").is_ok());
    }

    /// `DirectDfmQuery` delegates `delete_all_faults` correctly.
    #[test]
    fn direct_query_delete_all_faults() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let mut processor = FaultRecordProcessor::new(
            Arc::clone(&storage),
            Arc::clone(&registry),
            make_cycle_tracker(),
        );
        let path = make_path("test_entity");

        let record = make_record(
            FaultId::Text(to_static_short_string("fault_a").unwrap()),
            LifecycleStage::Failed,
        );
        processor.process_record(&path, &record);

        let query = make_direct_query(storage, registry);
        assert!(query.delete_all_faults("test_entity").is_ok());
    }

    /// `DirectDfmQuery` returns `BadArgument` for unknown path.
    #[test]
    fn direct_query_bad_path() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let query = make_direct_query(storage, registry);
        assert_eq!(query.get_all_faults("nonexistent"), Err(Error::BadArgument));
    }

    /// `DfmQueryApi` is object-safe - can be used as Box<dyn>.
    #[test]
    fn trait_is_object_safe() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_text_registry();
        let boxed: Box<dyn DfmQueryApi> = Box::new(make_direct_query(storage, registry));
        // Just verify it compiles and can be called
        let _ = boxed.get_all_faults("test_entity");
    }
}
