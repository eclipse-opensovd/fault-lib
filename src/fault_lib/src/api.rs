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

use crate::enabling_condition::{
    EnablingCondition, EnablingConditionCallback, EnablingConditionError, EnablingConditionManager,
    FaultMonitor,
};
use crate::fault_manager_sink::SinkInitError;
use crate::{FaultSinkApi, LogHook, catalog::FaultCatalog, fault_manager_sink::FaultManagerSink};
use alloc::sync::{Arc, Weak};
use common::enabling_condition::EnablingConditionStatus;
use common::sink_error::SinkError;
use std::sync::OnceLock;

/// Consolidated global state — single `OnceLock` prevents partial initialization.
struct FaultApiState {
    sink: Weak<dyn FaultSinkApi>,
    catalog: Weak<FaultCatalog>,
    ec_manager: Weak<EnablingConditionManager>,
}

static STATE: OnceLock<FaultApiState> = OnceLock::new();
static LOG_HOOK: OnceLock<Arc<dyn LogHook>> = OnceLock::new();
/// Serializes the entire `try_new` sequence (hash check + STATE.set) to
/// prevent a TOCTOU race where two threads could both pass the hash check
/// against different catalogs before either commits to STATE.
static INIT_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Error type for `FaultApi` initialization failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InitError {
    #[error("FaultApi already initialized")]
    AlreadyInitialized,

    #[error("catalog hash verification failed: {0}")]
    CatalogVerification(#[from] SinkError),

    #[error("IPC sink initialization failed: {0}")]
    SinkInit(#[from] SinkInitError),
}

/// `FaultApi` is the long-lived handle that wires a sink and logger together.
#[derive(Clone)]
pub struct FaultApi {
    _fault_sink: Arc<dyn FaultSinkApi>,
    _fault_catalog: Arc<FaultCatalog>,
    _ec_manager: Arc<EnablingConditionManager>,
}

impl FaultApi {
    /// Initialize the `FaultApi` singleton.
    ///
    /// # Errors
    ///
    /// Returns `InitError::AlreadyInitialized` if called more than once.
    /// Returns `InitError::CatalogVerification` if catalog hash check fails.
    #[tracing::instrument(skip(catalog))]
    pub fn try_new(catalog: FaultCatalog) -> Result<FaultApi, InitError> {
        let _guard = INIT_GUARD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Fast path: already initialized.
        if STATE.get().is_some() {
            return Err(InitError::AlreadyInitialized);
        }

        let catalog = Arc::new(catalog);

        // EC manager created first (without sink - circular dependency).
        // The IpcWorker receives a valid Weak<EnablingConditionManager> so
        // it can forward DFM notifications to local monitors.
        let ec_manager = Arc::new(EnablingConditionManager::new());

        // Sink created with valid Weak to EC manager.
        let concrete_sink = FaultManagerSink::with_ec_manager(Arc::downgrade(&ec_manager))?;

        // Validate catalog hash BEFORE committing to global state.
        // On failure the caller can retry with different input.
        if !concrete_sink.check_catalog_hash(&catalog)? {
            return Err(InitError::CatalogVerification(SinkError::Other(
                "catalog hash mismatch with DFM".into(),
            )));
        }

        let sink: Arc<dyn FaultSinkApi> = Arc::new(concrete_sink);

        // Wire sink back to EC manager (resolves circular dependency).
        ec_manager.set_sink(Arc::downgrade(&sink));

        // Atomic commit - either all three refs are stored or none.
        // The INIT_GUARD ensures no other thread can race between the
        // hash check above and this STATE.set() call.
        STATE
            .set(FaultApiState {
                sink: Arc::downgrade(&sink),
                catalog: Arc::downgrade(&catalog),
                ec_manager: Arc::downgrade(&ec_manager),
            })
            .map_err(|_| InitError::AlreadyInitialized)?;

        Ok(FaultApi {
            _fault_sink: sink,
            _fault_catalog: catalog,
            _ec_manager: ec_manager,
        })
    }

    /// Initialize the `FaultApi` singleton, panicking on error.
    ///
    /// # Panics
    ///
    /// Panics if `FaultApi` is already initialized or catalog verification fails.
    /// Use [`try_new`](Self::try_new) for the fallible version.
    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn new(catalog: FaultCatalog) -> FaultApi {
        Self::try_new(catalog).expect("FaultApi initialization failed")
    }

    /// Get the fault sink, if `FaultApi` is initialized and not dropped.
    ///
    /// # Errors
    ///
    /// Returns `SinkError::Other` if `FaultApi` was never initialized or has been dropped.
    pub(crate) fn try_get_fault_sink() -> Result<Arc<dyn FaultSinkApi>, SinkError> {
        STATE
            .get()
            .ok_or(SinkError::Other(alloc::borrow::Cow::Borrowed(
                "FaultApi not initialized",
            )))?
            .sink
            .upgrade()
            .ok_or(SinkError::Other(alloc::borrow::Cow::Borrowed(
                "FaultApi has been dropped",
            )))
    }

    /// Get the fault sink. Panics if `FaultApi` is not initialized or has been dropped.
    ///
    /// Use [`try_get_fault_sink`](Self::try_get_fault_sink) for the fallible version.
    #[allow(dead_code, clippy::expect_used)]
    pub(crate) fn get_fault_sink() -> Arc<dyn FaultSinkApi> {
        Self::try_get_fault_sink().expect("FaultApi not initialized or dropped")
    }

    /// Get the fault catalog, if `FaultApi` is initialized and not dropped.
    pub fn try_get_fault_catalog() -> Option<Arc<FaultCatalog>> {
        STATE.get().and_then(|s| s.catalog.upgrade())
    }

    /// Get the fault catalog. Panics if `FaultApi` is not initialized or has been dropped.
    ///
    /// # Panics
    ///
    /// Panics if `FaultApi` was never initialized or has been dropped.
    /// Use [`try_get_fault_catalog`](Self::try_get_fault_catalog) for the fallible version.
    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn get_fault_catalog() -> Arc<FaultCatalog> {
        Self::try_get_fault_catalog().expect("FaultApi not initialized or dropped")
    }

    /// Register a log hook for fault reporting observability.
    ///
    /// Can only be called once. Call before creating Reporters so they
    /// pick up the hook during construction.
    ///
    /// # Errors
    ///
    /// Returns the hook back if one was already registered.
    pub fn set_log_hook(hook: Arc<dyn LogHook>) -> Result<(), Arc<dyn LogHook>> {
        LOG_HOOK.set(hook)
    }

    /// Try to get the registered log hook.
    ///
    /// Returns `None` if no hook was registered via [`set_log_hook`](Self::set_log_hook).
    pub fn try_get_log_hook() -> Option<Arc<dyn LogHook>> {
        LOG_HOOK.get().cloned()
    }

    // ========================================================================
    // Enabling Conditions API
    // ========================================================================

    /// Get the enabling condition manager, if `FaultApi` is initialized and not dropped.
    pub(crate) fn try_get_ec_manager() -> Option<Arc<EnablingConditionManager>> {
        STATE.get().and_then(|s| s.ec_manager.upgrade())
    }

    /// Register a new enabling condition provider.
    ///
    /// Returns a handle that the caller uses to report status changes.
    /// The condition is registered with the DFM via IPC and starts in
    /// [`Inactive`](EnablingConditionStatus::Inactive) state.
    ///
    /// # Errors
    ///
    /// - `EnablingConditionError::AlreadyRegistered` if the entity is already registered.
    /// - `EnablingConditionError::NotInitialized` if `FaultApi` was not initialized.
    /// - `EnablingConditionError::EntityTooLong` if the entity name exceeds IPC limits.
    pub fn get_enabling_condition(
        entity: &str,
    ) -> Result<EnablingCondition, EnablingConditionError> {
        let manager = Self::try_get_ec_manager().ok_or(EnablingConditionError::NotInitialized)?;
        manager.register_condition(entity)
    }

    /// Create a fault monitor that watches one or more enabling conditions.
    ///
    /// When any of the specified conditions changes status, the callback
    /// is invoked with the condition ID and new status.
    ///
    /// The monitor automatically unregisters when dropped.
    ///
    /// # Errors
    ///
    /// - `EnablingConditionError::NotInitialized` if `FaultApi` was not initialized.
    pub fn create_fault_monitor(
        condition_ids: &[&str],
        callback: impl EnablingConditionCallback,
    ) -> Result<FaultMonitor, EnablingConditionError> {
        let manager = Self::try_get_ec_manager().ok_or(EnablingConditionError::NotInitialized)?;
        let ids: Vec<String> = condition_ids.iter().map(|s| (*s).to_string()).collect();
        manager.register_monitor(ids, Arc::new(callback))
    }

    /// Get the current status of a registered enabling condition.
    ///
    /// Returns `None` if `FaultApi` is not initialized or the condition
    /// is not registered.
    #[must_use]
    pub fn get_enabling_condition_status(entity: &str) -> Option<EnablingConditionStatus> {
        Self::try_get_ec_manager()?.get_status(entity)
    }
}

// ============================================================================
// Unit tests — cover the pre-initialisation error paths that do NOT
//              require a live iceoryx2 daemon.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    // ---- try_get_fault_sink (before init) ----

    #[test]
    fn try_get_fault_sink_before_init_returns_error() {
        // STATE is process-global, so this only works reliably if no other
        // test has successfully initialised FaultApi.  Because the real
        // init path requires iceoryx2, the singleton is always empty in
        // plain `cargo test`.
        let result = FaultApi::try_get_fault_sink();
        assert!(result.is_err());
    }

    // ---- try_get_fault_catalog (before init) ----

    #[test]
    fn try_get_fault_catalog_before_init_returns_none() {
        assert!(FaultApi::try_get_fault_catalog().is_none());
    }

    // ---- try_get_ec_manager (before init) ----

    #[test]
    fn try_get_ec_manager_before_init_returns_none() {
        assert!(FaultApi::try_get_ec_manager().is_none());
    }

    // ---- set_log_hook / try_get_log_hook ----

    #[test]
    fn try_get_log_hook_returns_none_when_unset() {
        // LOG_HOOK is also process-global.  If this is the first test to
        // run, it will be empty.  If another test already set it, skip.
        // We cannot reset a OnceLock, so this is best-effort.
        let _hook = FaultApi::try_get_log_hook();
        // Just verify it does not panic.
    }

    // ---- InitError Display ----

    #[test]
    fn init_error_already_initialized_display() {
        let err = InitError::AlreadyInitialized;
        assert_eq!(format!("{err}"), "FaultApi already initialized");
    }

    #[test]
    fn init_error_catalog_verification_display() {
        let err = InitError::CatalogVerification(SinkError::TransportDown);
        let msg = format!("{err}");
        assert!(msg.contains("catalog hash verification failed"));
    }

    // ---- Enabling condition error paths ----

    #[test]
    fn get_enabling_condition_before_init_returns_error() {
        let result = FaultApi::get_enabling_condition("test_ec");
        assert!(matches!(
            result,
            Err(EnablingConditionError::NotInitialized)
        ));
    }

    #[test]
    fn get_enabling_condition_status_before_init_returns_none() {
        assert!(FaultApi::get_enabling_condition_status("anything").is_none());
    }
}
