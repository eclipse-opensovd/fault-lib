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

//! Core processing logic for incoming fault reports.
//!
//! [`FaultRecordProcessor`] receives [`FaultRecord`](common::fault::FaultRecord)
//! messages from the IPC layer and applies:
//! - Catalog hash verification (reporter ↔ DFM agreement).
//! - Per-source debounce filtering.
//! - Lifecycle-stage transitions and aging-state management.
//! - Persistence of confirmed fault state via [`SovdFaultStateStorage`].

use alloc::sync::Arc;
use std::{collections::HashMap, sync::RwLock, time::Instant};

use common::{
    debounce::Debounce,
    fault,
    fault::{ComplianceTag, FaultId, LifecycleStage},
    types::{LongString, Sha256Vec},
};
use tracing::{error, info, trace, warn};

use crate::{
    aging_manager::{AgingManager, AgingState},
    fault_catalog_registry::FaultCatalogRegistry,
    operation_cycle::OperationCycleTracker,
    sovd_fault_storage::SovdFaultStateStorage,
};

/// Unique key for per-fault debounce state in DFM.
/// Combines source path (IPC identity) and fault ID so that each
/// reporter app has independent debounce state (Option A — per-source).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct FaultKey {
    pub source: String,
    pub fault_id: FaultId,
}

impl FaultKey {
    pub fn new(source: &str, fault_id: &FaultId) -> Self {
        Self {
            source: source.to_string(),
            fault_id: fault_id.clone(),
        }
    }
}

pub struct FaultRecordProcessor<S: SovdFaultStateStorage> {
    storage: Arc<S>,
    catalog_registry: Arc<FaultCatalogRegistry>,
    /// Per-source, per-fault debounce state. Lazily populated from catalog on first event.
    debouncers: HashMap<FaultKey, Box<dyn Debounce>>,
    /// Tracks last confirmed (post-debounce) lifecycle stage per fault key.
    /// Used to detect transitions that should reset the debouncer.
    last_stages: HashMap<FaultKey, LifecycleStage>,
    /// Evaluates fault aging/reset policies against operation cycle counters.
    aging_manager: AgingManager,
    /// Per-fault aging state tracking cycle counts and timestamps.
    /// Keyed by `(source, fault_id)` — same granularity as debounce state.
    aging_states: HashMap<FaultKey, AgingState>,
    /// Shared operation cycle tracker for aging state snapshots.
    cycle_tracker: Arc<RwLock<OperationCycleTracker>>,
}

impl<S: SovdFaultStateStorage> FaultRecordProcessor<S> {
    pub fn new(
        storage: Arc<S>,
        catalog_registry: Arc<FaultCatalogRegistry>,
        cycle_tracker: Arc<RwLock<OperationCycleTracker>>,
    ) -> Self {
        let aging_manager = AgingManager::new(Arc::clone(&cycle_tracker));
        Self {
            storage,
            catalog_registry,
            debouncers: HashMap::new(),
            last_stages: HashMap::new(),
            aging_manager,
            aging_states: HashMap::new(),
            cycle_tracker,
        }
    }

    #[tracing::instrument(skip(self, record))]
    pub fn process_record(&mut self, path: &LongString, record: &fault::FaultRecord) {
        let path_str = path.to_string();
        let key = FaultKey::new(&path_str, &record.id);

        // Validate lifecycle transition (warn-only, does not block for backward compat).
        if let Some(last_stage) = self.last_stages.get(&key)
            && !last_stage.is_valid_transition(&record.lifecycle_stage)
        {
            warn!(
                "Invalid lifecycle transition {:?} → {:?} for fault {:?} from {:?}",
                last_stage, record.lifecycle_stage, record.id, path_str
            );
        }

        // Handle lifecycle transition — reset debounce on Passed→Failed etc.
        self.handle_lifecycle_transition(&key, record.lifecycle_stage);

        // Always track incoming lifecycle stage for transition detection.
        // Updated BEFORE debounce check so a transition is consumed once
        // and does not repeatedly reset the debouncer on suppressed events.
        self.last_stages.insert(key.clone(), record.lifecycle_stage);

        // Apply manager-side debounce filter
        if !self.check_debounce(&key, &record.id) {
            trace!(
                "Fault {:?} from {:?} suppressed by manager-side debounce",
                record.id, path_str
            );
            return;
        }

        // Read existing state to preserve counters and latched flags across events.
        let mut state = self
            .storage
            .get(&path_str, &record.id)
            .ok()
            .flatten()
            .unwrap_or_default();

        // Resolve whether this fault has an aging/reset policy configured.
        let reset_policy = self.lookup_reset_policy(&key, &record.id);

        match record.lifecycle_stage {
            fault::LifecycleStage::Failed => {
                state.test_failed = true;
                state.confirmed_dtc = true;
                state.pending_dtc = false;
                state.test_failed_this_operation_cycle = true;
                state.test_failed_since_last_clear = true;

                // ISO 14229: WIR driven by descriptor compliance flags
                if self.is_warning_indicator_relevant(&key, &record.id) {
                    state.warning_indicator_requested = true;
                }

                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                state.record_occurrence(now_secs);

                self.mark_aging_active(&key);
            }
            fault::LifecycleStage::Passed => {
                state.test_failed = false;
                state.pending_dtc = false;

                if reset_policy.is_some() {
                    // ISO 14229 aging: confirmed_dtc stays latched until
                    // aging conditions are met. Only test_failed clears.
                } else {
                    state.confirmed_dtc = false;
                }
            }
            fault::LifecycleStage::PreFailed => {
                state.test_failed = true;
                state.pending_dtc = true;
                state.test_failed_this_operation_cycle = true;
                state.test_failed_since_last_clear = true;

                if self.is_warning_indicator_relevant(&key, &record.id) {
                    state.warning_indicator_requested = true;
                }

                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                state.record_occurrence(now_secs);

                // Conservative (automotive-safe): any sign of fault activity
                // keeps the DTC alive. A fault oscillating between PreFailed
                // and PrePassed should not be silently aged out.
                self.mark_aging_active(&key);
            }
            fault::LifecycleStage::PrePassed => {
                state.test_failed = false;
                state.pending_dtc = false;
            }
            fault::LifecycleStage::NotTested => {
                state.test_not_completed_this_operation_cycle = true;
                state.test_not_completed_since_last_clear = true;
            }
        }

        // Evaluate aging reset: if the fault is no longer active and the
        // reset policy trigger is satisfied, clear the latched DTC flags.
        if let Some(policy) = &reset_policy
            && let Some(aging_state) = self.aging_states.get_mut(&key)
            && self.aging_manager.should_reset(policy, aging_state)
        {
            info!(
                "Aging reset triggered for fault {:?} from {:?}",
                record.id, path_str
            );
            self.aging_manager.apply_reset(aging_state, &mut state);
        }

        // Preserve diagnostic env_data from failure stages — a Passed/PrePassed
        // event must not overwrite the env_data captured during the fault.
        if matches!(
            record.lifecycle_stage,
            fault::LifecycleStage::Failed | fault::LifecycleStage::PreFailed
        ) {
            state.env_data = record
                .env_data
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        }
        match self.storage.put(&path_str, &record.id, state) {
            Ok(()) => info!("Fault ID {:?} stored", record.id),
            Err(e) => error!("Failed to store fault ID {:?}: {}", record.id, e),
        }
    }

    /// Returns `true` if the event should be processed (passes debounce),
    /// `false` if it should be suppressed.
    fn check_debounce(&mut self, key: &FaultKey, fault_id: &FaultId) -> bool {
        match self.get_or_create_debouncer(key, fault_id) {
            Some(debouncer) => debouncer.on_event(Instant::now()),
            None => true, // No debounce configured — pass through
        }
    }

    /// Lazily looks up the debouncer for a fault key, creating it from
    /// the catalog's `manager_side_debounce` config on first access.
    /// Returns `None` when no manager-side debounce is configured.
    fn get_or_create_debouncer(
        &mut self,
        key: &FaultKey,
        fault_id: &FaultId,
    ) -> Option<&mut Box<dyn Debounce>> {
        if self.debouncers.contains_key(key) {
            return self.debouncers.get_mut(key);
        }

        // Look up descriptor from catalog
        let catalog = self.catalog_registry.get(&key.source)?;
        let descriptor = catalog.descriptor(fault_id)?;

        // Check if manager-side debounce is configured
        let debounce_mode = descriptor.manager_side_debounce?;

        let debouncer = debounce_mode.into_debouncer();
        self.debouncers.insert(key.clone(), debouncer);
        self.debouncers.get_mut(key)
    }

    /// Detects lifecycle transitions that should reset the debouncer,
    /// e.g. when a fault clears (Passed) and then re-occurs (Failed).
    fn handle_lifecycle_transition(&mut self, key: &FaultKey, new_stage: LifecycleStage) {
        if let Some(last_stage) = self.last_stages.get(key)
            && Self::should_reset_debounce(*last_stage, new_stage)
            && let Some(debouncer) = self.debouncers.get_mut(key)
        {
            trace!(
                "Resetting manager-side debounce for {:?} on {:?} → {:?} transition",
                key.fault_id, last_stage, new_stage
            );
            debouncer.reset(Instant::now());
        }
    }

    /// Transitions that warrant a debounce reset: the fault was cleared
    /// and is now re-entering a failure state.
    fn should_reset_debounce(last: LifecycleStage, new: LifecycleStage) -> bool {
        use LifecycleStage::{Failed, NotTested, Passed, PreFailed};
        matches!(
            (last, new),
            (Passed | NotTested, Failed) | (Passed, PreFailed)
        )
    }

    /// Mark the aging state for a fault as active (failure re-occurred).
    /// Creates the aging state entry if it doesn't exist yet.
    fn mark_aging_active(&mut self, key: &FaultKey) {
        // Recover from RwLock poisoning — data integrity is more important
        // than propagating a panic from an unrelated thread.
        let tracker = self
            .cycle_tracker
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let aging_state = self.aging_states.entry(key.clone()).or_default();
        aging_state.mark_active(&tracker);
    }

    /// Check whether the fault descriptor flags warrant a warning indicator.
    ///
    /// ISO 14229: `warning_indicator_requested` is set when the descriptor
    /// carries `SafetyCritical` or `EmissionRelevant` compliance tags.
    fn is_warning_indicator_relevant(&self, key: &FaultKey, fault_id: &FaultId) -> bool {
        let Some(catalog) = self.catalog_registry.get(&key.source) else {
            return false;
        };
        let Some(descriptor) = catalog.descriptor(fault_id) else {
            return false;
        };
        descriptor.compliance.iter().any(|tag| {
            matches!(
                tag,
                ComplianceTag::SafetyCritical | ComplianceTag::EmissionRelevant
            )
        })
    }

    /// Handle a "clear DTC" operation for a specific path.
    ///
    /// ISO 14229: resets `*_since_last_clear` flags for all faults at this path.
    /// This is triggered by diagnostic tool commands (UDS $14 `ClearDTC`).
    pub fn clear_dtc(&self, path: &str) {
        match self.storage.get_all(path) {
            Ok(faults) => {
                for (fault_id, mut state) in faults {
                    state.test_failed_since_last_clear = false;
                    state.test_not_completed_since_last_clear = false;
                    if let Err(e) = self.storage.put(path, &fault_id, state) {
                        error!("Failed to clear DTC flags for {fault_id:?}: {e}");
                    }
                }
                info!("Clear DTC completed for path: {path}");
            }
            Err(e) => error!("Failed to read faults for clear DTC on {path}: {e}"),
        }
    }

    /// Handle a new operation cycle boundary.
    ///
    /// ISO 14229: resets `*_this_operation_cycle` flags for all faults at this path.
    /// Called when the operation cycle tracker detects a cycle transition.
    pub fn on_new_operation_cycle(&self, path: &str) {
        match self.storage.get_all(path) {
            Ok(faults) => {
                for (fault_id, mut state) in faults {
                    state.test_failed_this_operation_cycle = false;
                    state.test_not_completed_this_operation_cycle = false;
                    if let Err(e) = self.storage.put(path, &fault_id, state) {
                        error!("Failed to reset cycle flags for {fault_id:?}: {e}");
                    }
                }
            }
            Err(e) => error!("Failed to read faults for cycle boundary on {path}: {e}"),
        }
    }

    /// Look up `manager_side_reset` policy from the fault catalog for this key.
    fn lookup_reset_policy(
        &self,
        key: &FaultKey,
        fault_id: &FaultId,
    ) -> Option<common::config::ResetPolicy> {
        let catalog = self.catalog_registry.get(&key.source)?;
        let descriptor = catalog.descriptor(fault_id)?;
        descriptor.manager_side_reset.clone()
    }

    /// Provide read access to the shared operation cycle tracker,
    /// allowing external callers (e.g. `DiagnosticFaultManager`) to
    /// advance operation cycles.
    #[must_use]
    pub fn cycle_tracker(&self) -> &Arc<RwLock<OperationCycleTracker>> {
        &self.cycle_tracker
    }

    pub fn check_hash_sum(&self, path: &LongString, hash_sum: &Sha256Vec) -> bool {
        if let Some(catalog) = self.catalog_registry.get(&path.to_string()) {
            let ret = catalog.config_hash() == hash_sum.to_vec();
            if !ret {
                error!("Fault catalog hash sum error for {:?}", path.to_string());
                error!("Expected {:?}", catalog.config_hash());
                error!("Received {:?}", hash_sum.to_vec());
            }
            ret
        } else {
            error!("Catalog hash sum entity {:?} not found ", path.to_string());
            false
        }
    }
}

#[cfg(test)]
mod processor_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::sync::Arc;

    use common::fault::*;

    use crate::{
        dfm_test_utils::*, fault_record_processor::FaultRecordProcessor,
        sovd_fault_storage::SovdFaultStateStorage,
    };

    fn make_processor(
        storage: Arc<InMemoryStorage>,
        registry: Arc<crate::fault_catalog_registry::FaultCatalogRegistry>,
    ) -> FaultRecordProcessor<InMemoryStorage> {
        FaultRecordProcessor::new(storage, registry, make_cycle_tracker())
    }

    /// Processor correctly handles Failed lifecycle stage.
    #[test]
    fn processor_handles_failed_stage() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);

        let record = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        let path = make_path("test_entity");
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(state.is_some(), "Failed stage should store state");

        let state = state.unwrap();
        assert!(state.test_failed, "Failed should set test_failed=true");
        assert!(state.confirmed_dtc, "Failed should set confirmed_dtc=true");
    }

    /// Processor correctly handles Passed lifecycle stage.
    #[test]
    fn processor_handles_passed_stage() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);

        let record = make_record(FaultId::Numeric(42), LifecycleStage::Passed);
        let path = make_path("test_entity");
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(state.is_some(), "Passed stage should store state");

        let state = state.unwrap();
        assert!(!state.test_failed, "Passed should set test_failed=false");
        assert!(
            !state.confirmed_dtc,
            "Passed should set confirmed_dtc=false"
        );
    }

    /// Processor handles transition from Failed to Passed.
    #[test]
    fn processor_handles_failed_to_passed_transition() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        // First: Failed
        let record_failed = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        processor.process_record(&path, &record_failed);

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(state.test_failed);
        assert!(state.confirmed_dtc);

        // Then: Passed
        let record_passed = make_record(FaultId::Numeric(42), LifecycleStage::Passed);
        processor.process_record(&path, &record_passed);

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(!state.test_failed);
        assert!(!state.confirmed_dtc);
    }

    /// Processor handles `PreFailed` stage correctly.
    #[test]
    fn processor_handles_prefailed_stage() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);

        let record = make_record(FaultId::Numeric(42), LifecycleStage::PreFailed);
        let path = make_path("test_entity");
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(state.is_some(), "PreFailed stage should store state");
        let state = state.unwrap();
        assert!(state.test_failed, "PreFailed should set test_failed=true");
        assert!(state.pending_dtc, "PreFailed should set pending_dtc=true");
        assert!(
            !state.confirmed_dtc,
            "PreFailed should NOT set confirmed_dtc"
        );
    }

    /// Processor handles `PrePassed` stage correctly.
    #[test]
    fn processor_handles_prepassed_stage() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);

        let record = make_record(FaultId::Numeric(42), LifecycleStage::PrePassed);
        let path = make_path("test_entity");
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(state.is_some(), "PrePassed stage should store state");
        let state = state.unwrap();
        assert!(!state.test_failed, "PrePassed should set test_failed=false");
        assert!(!state.pending_dtc, "PrePassed should set pending_dtc=false");
    }

    /// Processor handles `NotTested` stage correctly.
    #[test]
    fn processor_handles_nottested_stage() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry();
        let mut processor = make_processor(Arc::clone(&storage), registry);

        let record = make_record(FaultId::Numeric(42), LifecycleStage::NotTested);
        let path = make_path("test_entity");
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(state.is_some(), "NotTested stage should store state");
        let state = state.unwrap();
        assert!(
            state.test_not_completed_this_operation_cycle,
            "NotTested should set test_not_completed_this_operation_cycle=true"
        );
    }
}

#[cfg(test)]
mod debounce_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::{sync::Arc, time::Duration};

    use common::{debounce::DebounceMode, fault::*};

    use crate::{
        dfm_test_utils::*, fault_record_processor::FaultRecordProcessor,
        sovd_fault_storage::SovdFaultStateStorage as _,
    };

    fn make_processor(
        storage: Arc<InMemoryStorage>,
        registry: Arc<crate::fault_catalog_registry::FaultCatalogRegistry>,
    ) -> FaultRecordProcessor<InMemoryStorage> {
        FaultRecordProcessor::new(storage, registry, make_cycle_tracker())
    }

    /// DFM applies manager-side debounce: first (min_count-1) events are
    /// suppressed, the min_count-th event fires and updates storage.
    #[test]
    fn dfm_applies_manager_side_debounce() {
        let fault_id = FaultId::Numeric(100);
        let debounce = DebounceMode::CountWithinWindow {
            min_count: 3,
            window: Duration::from_secs(10).into(),
        };
        let registry = make_debounce_registry("test_entity", fault_id.clone(), debounce);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        // Events 1 and 2: suppressed (count < min_count)
        let record = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &record);
        assert!(
            storage.get("test_entity", &fault_id).unwrap().is_none(),
            "First event should be suppressed by debounce"
        );

        processor.process_record(&path, &record);
        assert!(
            storage.get("test_entity", &fault_id).unwrap().is_none(),
            "Second event should be suppressed by debounce"
        );

        // Event 3: fires (count == min_count)
        processor.process_record(&path, &record);
        let state = storage.get("test_entity", &fault_id).unwrap();
        assert!(
            state.is_some(),
            "Third event should pass debounce and update storage"
        );
        let state = state.unwrap();
        assert!(state.test_failed);
        assert!(state.confirmed_dtc);
    }

    /// When `manager_side_debounce` is None, every event passes through.
    #[test]
    fn dfm_passes_through_without_debounce_config() {
        let storage = Arc::new(InMemoryStorage::new());
        let registry = make_registry(); // No debounce configured
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &FaultId::Numeric(42)).unwrap();
        assert!(
            state.is_some(),
            "Event should pass through when no debounce configured"
        );
        assert!(state.unwrap().test_failed);
    }

    /// Debounce resets on lifecycle transition (Passed -> Failed).
    /// After a Passed event fires, subsequent Failed events restart the counter.
    #[test]
    fn dfm_debounce_resets_on_lifecycle_transition() {
        let fault_id = FaultId::Numeric(200);
        let debounce = DebounceMode::CountWithinWindow {
            min_count: 3,
            window: Duration::from_secs(60).into(),
        };
        let registry = make_debounce_registry("test_entity", fault_id.clone(), debounce);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        let passed = make_record(fault_id.clone(), LifecycleStage::Passed);

        // 2 Failed events: suppressed (count 1, 2)
        processor.process_record(&path, &failed);
        processor.process_record(&path, &failed);
        assert!(
            storage.get("test_entity", &fault_id).unwrap().is_none(),
            "First two Failed events should be suppressed"
        );

        // Passed event: this is the 3rd event, reaches min_count -> fires
        processor.process_record(&path, &passed);
        let state = storage.get("test_entity", &fault_id).unwrap();
        assert!(state.is_some(), "Passed event (3rd) should fire");
        assert!(
            !state.unwrap().test_failed,
            "Passed should clear test_failed"
        );

        // Now Failed again: lifecycle transition (Passed->Failed) resets debouncer
        // After reset, counter restarts: event 1 -> suppressed
        processor.process_record(&path, &failed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.test_failed,
            "First Failed after reset should be suppressed; Passed state remains"
        );

        // Events 2 and 3 after reset
        processor.process_record(&path, &failed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.test_failed,
            "Second Failed after reset should still be suppressed"
        );

        processor.process_record(&path, &failed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.test_failed,
            "Third Failed after reset should fire (counter reached min_count)"
        );
        assert!(state.confirmed_dtc);
    }

    /// Debounce state is independent per source (per-app).
    /// app1 reaching `min_count` does not affect app2's counter.
    #[test]
    fn dfm_debounce_is_per_source() {
        let fault_id = FaultId::Numeric(300);
        let debounce = DebounceMode::CountWithinWindow {
            min_count: 2,
            window: Duration::from_secs(10).into(),
        };
        let registry = make_two_source_debounce_registry(fault_id.clone(), debounce);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path1 = make_path("app1");
        let path2 = make_path("app2");

        let record = make_record(fault_id.clone(), LifecycleStage::Failed);

        // app1: event 1 -> suppressed
        processor.process_record(&path1, &record);
        assert!(
            storage.get("app1", &fault_id).unwrap().is_none(),
            "app1 first event should be suppressed"
        );

        // app2: event 1 -> suppressed
        processor.process_record(&path2, &record);
        assert!(
            storage.get("app2", &fault_id).unwrap().is_none(),
            "app2 first event should be suppressed"
        );

        // app1: event 2 -> fires (count == min_count for app1)
        processor.process_record(&path1, &record);
        assert!(
            storage.get("app1", &fault_id).unwrap().is_some(),
            "app1 second event should fire (reached min_count)"
        );
        // app2 should still be suppressed
        assert!(
            storage.get("app2", &fault_id).unwrap().is_none(),
            "app2 should still be suppressed (only 1 event)"
        );

        // app2: event 2 -> fires independently
        processor.process_record(&path2, &record);
        assert!(
            storage.get("app2", &fault_id).unwrap().is_some(),
            "app2 second event should fire independently"
        );
    }

    /// `EdgeWithCooldown` debounce: first event fires, subsequent within cooldown suppressed.
    #[test]
    fn dfm_applies_edge_with_cooldown_debounce() {
        let fault_id = FaultId::Numeric(400);
        let debounce = DebounceMode::EdgeWithCooldown {
            cooldown: Duration::from_secs(60).into(),
        };
        let registry = make_debounce_registry("test_entity", fault_id.clone(), debounce);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(fault_id.clone(), LifecycleStage::Failed);

        // First event: passes (edge trigger)
        processor.process_record(&path, &record);
        assert!(
            storage.get("test_entity", &fault_id).unwrap().is_some(),
            "First event should pass through (edge trigger)"
        );

        // Overwrite storage to detect if second event updates it
        storage.delete("test_entity", &fault_id).unwrap();

        // Second event within cooldown: suppressed
        processor.process_record(&path, &record);
        assert!(
            storage.get("test_entity", &fault_id).unwrap().is_none(),
            "Second event within cooldown should be suppressed"
        );
    }
}

#[cfg(test)]
mod iso14229_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::std_instead_of_core,
        clippy::std_instead_of_alloc,
        clippy::arithmetic_side_effects
    )]

    use std::sync::Arc;

    use common::fault::*;

    use crate::{
        dfm_test_utils::*, fault_record_processor::FaultRecordProcessor,
        sovd_fault_storage::SovdFaultStateStorage,
    };

    fn make_processor(
        storage: Arc<InMemoryStorage>,
        registry: Arc<crate::fault_catalog_registry::FaultCatalogRegistry>,
    ) -> FaultRecordProcessor<InMemoryStorage> {
        FaultRecordProcessor::new(storage, registry, make_cycle_tracker())
    }

    // ========================================================================
    // warning_indicator_requested tests
    // ========================================================================

    /// Failed with `SafetyCritical` descriptor → `warning_indicator_requested` = true.
    #[test]
    fn warning_indicator_set_for_safety_critical() {
        let fault_id = FaultId::Numeric(700);
        let compliance = ComplianceVec::try_from(&[ComplianceTag::SafetyCritical][..]).unwrap();
        let registry = make_compliance_registry("test_entity", fault_id.clone(), compliance);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.warning_indicator_requested,
            "SafetyCritical fault should set WIR"
        );
    }

    /// Failed with `EmissionRelevant` descriptor → `warning_indicator_requested` = true.
    #[test]
    fn warning_indicator_set_for_emission_relevant() {
        let fault_id = FaultId::Numeric(701);
        let compliance = ComplianceVec::try_from(&[ComplianceTag::EmissionRelevant][..]).unwrap();
        let registry = make_compliance_registry("test_entity", fault_id.clone(), compliance);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.warning_indicator_requested,
            "EmissionRelevant fault should set WIR"
        );
    }

    /// Failed with `SecurityRelevant` only → `warning_indicator_requested` = false.
    #[test]
    fn warning_indicator_not_set_for_security_only() {
        let fault_id = FaultId::Numeric(702);
        let compliance = ComplianceVec::try_from(&[ComplianceTag::SecurityRelevant][..]).unwrap();
        let registry = make_compliance_registry("test_entity", fault_id.clone(), compliance);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.warning_indicator_requested,
            "SecurityRelevant-only should not set WIR"
        );
    }

    /// `PreFailed` also sets `warning_indicator_requested`.
    #[test]
    fn warning_indicator_set_on_prefailed() {
        let fault_id = FaultId::Numeric(703);
        let compliance = ComplianceVec::try_from(&[ComplianceTag::SafetyCritical][..]).unwrap();
        let registry = make_compliance_registry("test_entity", fault_id.clone(), compliance);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        let record = make_record(fault_id.clone(), LifecycleStage::PreFailed);
        processor.process_record(&path, &record);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            state.warning_indicator_requested,
            "PreFailed should also set WIR for SafetyCritical"
        );
    }

    // ========================================================================
    // clear_dtc tests
    // ========================================================================

    /// `clear_dtc` resets *_`since_last_clear` flags.
    #[test]
    fn clear_dtc_resets_since_last_clear_flags() {
        let fault_id = FaultId::Numeric(710);
        let _ = fault_id;
        let registry = make_registry();
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        // Create some fault state with since_last_clear flags set
        let record = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(state.test_failed_since_last_clear);

        // Clear DTC
        processor.clear_dtc("test_entity");

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(
            !state.test_failed_since_last_clear,
            "clear_dtc should reset test_failed_since_last_clear"
        );
        assert!(
            !state.test_not_completed_since_last_clear,
            "clear_dtc should reset test_not_completed_since_last_clear"
        );
        // Other flags preserved
        assert!(state.test_failed, "clear_dtc should not affect test_failed");
    }

    // ========================================================================
    // on_new_operation_cycle tests
    // ========================================================================

    /// `on_new_operation_cycle` resets *_`this_operation_cycle` flags.
    #[test]
    fn new_operation_cycle_resets_this_cycle_flags() {
        let registry = make_registry();
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        // Create fault state with this_operation_cycle flags set
        let record = make_record(FaultId::Numeric(42), LifecycleStage::Failed);
        processor.process_record(&path, &record);

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(state.test_failed_this_operation_cycle);

        // New operation cycle
        processor.on_new_operation_cycle("test_entity");

        let state = storage
            .get("test_entity", &FaultId::Numeric(42))
            .unwrap()
            .unwrap();
        assert!(
            !state.test_failed_this_operation_cycle,
            "new cycle should reset test_failed_this_operation_cycle"
        );
        assert!(
            !state.test_not_completed_this_operation_cycle,
            "new cycle should reset test_not_completed_this_operation_cycle"
        );
        // Other flags preserved
        assert!(state.test_failed, "new cycle should not affect test_failed");
        assert!(
            state.confirmed_dtc,
            "new cycle should not affect confirmed_dtc"
        );
    }

    /// Full ISO 14229 DTC lifecycle: Failed → clear → new cycle → Failed.
    #[test]
    fn full_dtc_lifecycle_clear_then_new_cycle() {
        let fault_id = FaultId::Numeric(42);
        let compliance = ComplianceVec::try_from(&[ComplianceTag::SafetyCritical][..]).unwrap();
        let registry = make_compliance_registry("test_entity", fault_id.clone(), compliance);
        let storage = Arc::new(InMemoryStorage::new());
        let mut processor = make_processor(Arc::clone(&storage), registry);
        let path = make_path("test_entity");

        // Step 1: Failed
        let failed = make_record(fault_id.clone(), LifecycleStage::Failed);
        processor.process_record(&path, &failed);

        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(state.test_failed);
        assert!(state.test_failed_this_operation_cycle);
        assert!(state.test_failed_since_last_clear);
        assert!(state.warning_indicator_requested);

        // Step 2: Clear DTC
        processor.clear_dtc("test_entity");
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(!state.test_failed_since_last_clear, "cleared");
        assert!(
            state.test_failed_this_operation_cycle,
            "clear doesn't touch this_cycle"
        );

        // Step 3: New operation cycle
        processor.on_new_operation_cycle("test_entity");
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(
            !state.test_failed_this_operation_cycle,
            "reset by new cycle"
        );
        assert!(!state.test_failed_since_last_clear, "still cleared");

        // Step 4: Failed again
        processor.process_record(&path, &failed);
        let state = storage.get("test_entity", &fault_id).unwrap().unwrap();
        assert!(state.test_failed_this_operation_cycle);
        assert!(state.test_failed_since_last_clear);
        assert_eq!(state.occurrence_counter, 2);
    }
}
