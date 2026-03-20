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

//! Enabling condition provider handles and fault monitors.
//!
//! This module implements the enabling condition flow described in
//! `docs/puml/new_enable_condition.puml` (registration),
//! `docs/puml/enable_condition_ntf.puml` (remote notifications), and
//! `docs/puml/local_enable_condition_ntf.puml` (local notifications).
//!
//! # Architecture
//!
//! - [`EnablingCondition`]: Provider-side handle for reporting condition status.
//! - [`FaultMonitor`]: Consumer-side handle for receiving condition change notifications.
//! - `EnablingConditionManager`: Internal singleton that tracks condition state
//!   and dispatches notifications to registered monitors.

use alloc::sync::{Arc, Weak};
use core::{
    panic::AssertUnwindSafe,
    sync::atomic::{AtomicU64, Ordering},
};
use std::{collections::HashMap, panic::catch_unwind, sync::RwLock};

use common::{
    enabling_condition::EnablingConditionStatus,
    sink_error::SinkError,
    types::{DiagnosticEvent, ShortString},
};
use tracing::{debug, error, warn};

// ============================================================================
// Error types
// ============================================================================

/// Errors that can occur during enabling condition operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnablingConditionError {
    #[error("enabling condition '{0}' already registered")]
    AlreadyRegistered(String),
    #[error("FaultApi not initialized or dropped")]
    NotInitialized,
    #[error("entity name too long for IPC transport")]
    EntityTooLong,
    #[error("IPC transport error: {0}")]
    Transport(#[from] SinkError),
    #[error("internal error: {0}")]
    InternalError(String),
}

// ============================================================================
// Callback trait
// ============================================================================

/// Callback interface for enabling condition change notifications.
///
/// Implementations must be thread-safe and non-blocking. The callback
/// is invoked from the notification dispatch thread; long-running work
/// should be offloaded to a separate task/thread.
pub trait EnablingConditionCallback: Send + Sync + 'static {
    /// Called when an enabling condition status changes.
    ///
    /// `id` is the SOVD entity name of the condition.
    fn on_condition_change(&self, id: &str, status: EnablingConditionStatus);
}

/// Blanket implementation for closures.
impl<F> EnablingConditionCallback for F
where
    F: Fn(&str, EnablingConditionStatus) + Send + Sync + 'static,
{
    fn on_condition_change(&self, id: &str, status: EnablingConditionStatus) {
        self(id, status);
    }
}

// ============================================================================
// EnablingCondition (provider handle)
// ============================================================================

/// Handle for an enabling condition provider.
///
/// Created via [`FaultApi::get_enabling_condition`](crate::api::FaultApi::get_enabling_condition). The provider uses
/// this handle to report status changes (active/inactive), which are
/// forwarded to the DFM via IPC and to local [`FaultMonitor`] subscribers.
///
/// # Example
///
/// ```ignore
/// let ec = FaultApi::get_enabling_condition("vehicle.speed.valid")?;
/// ec.report_status(EnablingConditionStatus::Active)?;
/// ```
pub struct EnablingCondition {
    id: ShortString,
    manager: Arc<EnablingConditionManager>,
}

impl EnablingCondition {
    /// Report the current status of this enabling condition.
    ///
    /// Updates local state, notifies local monitors, and sends the
    /// status change to the DFM via IPC.
    ///
    /// # Errors
    ///
    /// Returns [`SinkError`] if the IPC transport fails.
    pub fn report_status(&self, status: EnablingConditionStatus) -> Result<(), SinkError> {
        self.manager.report_status(&self.id, status)
    }

    /// Get the identifier of this enabling condition.
    #[must_use]
    pub fn id(&self) -> &ShortString {
        &self.id
    }
}

// ============================================================================
// FaultMonitor (consumer handle)
// ============================================================================

/// Monitor handle for receiving enabling condition change notifications.
///
/// Created via [`FaultApi::create_fault_monitor`](crate::api::FaultApi::create_fault_monitor). When any of the
/// monitored enabling conditions changes status, the registered callback
/// is invoked.
///
/// The monitor automatically unregisters from the manager when dropped.
///
/// # Example
///
/// ```ignore
/// let monitor = FaultApi::create_fault_monitor(
///     &["vehicle.speed.valid", "engine.running"],
///     |id, status| println!("condition {} changed to {:?}", id, status),
/// )?;
/// // monitor lives as long as the variable; dropped → unregistered
/// ```
pub struct FaultMonitor {
    monitor_id: u64,
    manager: Arc<EnablingConditionManager>,
}

impl Drop for FaultMonitor {
    fn drop(&mut self) {
        self.manager.unregister_monitor(self.monitor_id);
    }
}

// ============================================================================
// Internal: MonitorEntry
// ============================================================================

struct MonitorEntry {
    id: u64,
    condition_ids: Vec<String>,
    callback: Arc<dyn EnablingConditionCallback>,
}

// ============================================================================
// EnablingConditionManager (internal singleton)
// ============================================================================

/// Monotonic counter for unique monitor IDs.
static MONITOR_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Internal manager that tracks enabling condition state and dispatches
/// notifications to registered fault monitors.
///
/// Stored as a global singleton via `OnceLock` in [`FaultApi`].
/// Thread-safe: all mutable state is behind `RwLock`.
pub(crate) struct EnablingConditionManager {
    /// Registered enabling conditions: entity → current status.
    conditions: RwLock<HashMap<String, EnablingConditionStatus>>,
    /// Registered monitors with their callbacks.
    monitors: RwLock<Vec<MonitorEntry>>,
    /// Weak reference to the sink for IPC communication.
    /// Behind `RwLock` to allow deferred wiring (sink created after manager
    /// due to circular Arc<sink> ↔ Arc<manager> dependency).
    sink: RwLock<Option<Weak<dyn crate::FaultSinkApi>>>,
}

impl EnablingConditionManager {
    /// Create a new manager without a sink (will be wired later via `set_sink`).
    pub(crate) fn new() -> Self {
        Self {
            conditions: RwLock::new(HashMap::new()),
            monitors: RwLock::new(Vec::new()),
            sink: RwLock::new(None),
        }
    }

    /// Create a new manager with a weak reference to the IPC sink.
    #[allow(dead_code)]
    pub(crate) fn with_sink(sink: Weak<dyn crate::FaultSinkApi>) -> Self {
        Self {
            conditions: RwLock::new(HashMap::new()),
            monitors: RwLock::new(Vec::new()),
            sink: RwLock::new(Some(sink)),
        }
    }

    /// Wire the sink after construction (resolves circular dependency).
    pub(crate) fn set_sink(&self, sink: Weak<dyn crate::FaultSinkApi>) {
        if let Ok(mut s) = self.sink.write() {
            *s = Some(sink);
        }
    }

    fn get_sink(&self) -> Option<Arc<dyn crate::FaultSinkApi>> {
        self.sink.read().ok()?.as_ref()?.upgrade()
    }

    /// Register a new enabling condition.
    ///
    /// Returns `Err` if the condition is already registered.
    pub(crate) fn register_condition(
        self: &Arc<Self>,
        entity: &str,
    ) -> Result<EnablingCondition, EnablingConditionError> {
        let id = ShortString::try_from(entity.as_bytes())
            .map_err(|_| EnablingConditionError::EntityTooLong)?;

        {
            let mut conditions = self.conditions.write().map_err(|e| {
                error!("conditions lock poisoned in register_condition: {e}");
                EnablingConditionError::InternalError(format!("conditions lock poisoned: {e}"))
            })?;

            if conditions.contains_key(entity) {
                return Err(EnablingConditionError::AlreadyRegistered(
                    entity.to_string(),
                ));
            }
            conditions.insert(entity.to_string(), EnablingConditionStatus::Inactive);
        }

        // Send registration to DFM via IPC (best-effort)
        if let Some(sink) = self.get_sink() {
            let event = DiagnosticEvent::EnablingConditionRegister(id);
            if let Err(e) = sink.send_event(event) {
                warn!("Failed to register enabling condition '{entity}' with DFM: {e:?}");
            }
        }

        debug!("Registered enabling condition: {entity}");
        Ok(EnablingCondition {
            id,
            manager: Arc::clone(self),
        })
    }

    /// Report a status change for an enabling condition.
    ///
    /// Updates local state, notifies local monitors, and sends to DFM.
    pub(crate) fn report_status(
        &self,
        id: &ShortString,
        status: EnablingConditionStatus,
    ) -> Result<(), SinkError> {
        let id_str = id.to_string();

        // Update local state
        {
            let mut conditions = self.conditions.write().map_err(|e| {
                error!("conditions lock poisoned in report_status: {e}");
                SinkError::Other(alloc::borrow::Cow::Owned(format!(
                    "conditions lock poisoned: {e}"
                )))
            })?;

            if let Some(current) = conditions.get_mut(&id_str) {
                if *current == status {
                    // No change — skip notification
                    return Ok(());
                }
                *current = status;
            } else {
                // Condition not registered locally but may be registered remotely
                conditions.insert(id_str.clone(), status);
            }
        }

        // Notify local monitors (lock released before callback invocation)
        self.dispatch_to_monitors(&id_str, status);

        // Send to DFM via IPC
        if let Some(sink) = self.get_sink() {
            let event = DiagnosticEvent::EnablingConditionStatusChange((*id, status));
            sink.send_event(event)?;
        }

        debug!("Enabling condition '{id_str}' status: {status:?}");
        Ok(())
    }

    /// Handle an incoming notification from DFM (remote status change).
    ///
    /// Called by the notification listener when a status change is
    /// received from DFM. Updates local state and dispatches to monitors.
    pub(crate) fn handle_remote_notification(
        &self,
        id: &ShortString,
        status: EnablingConditionStatus,
    ) {
        let id_str = id.to_string();

        // Update local state
        match self.conditions.write() {
            Ok(mut conditions) => {
                if let Some(current) = conditions.get_mut(&id_str) {
                    if *current == status {
                        return; // No change
                    }
                    *current = status;
                } else {
                    conditions.insert(id_str.clone(), status);
                }
            }
            Err(e) => {
                error!("conditions lock poisoned in handle_remote_notification: {e}");
                return;
            }
        }

        // Dispatch to monitors
        self.dispatch_to_monitors(&id_str, status);
        debug!("Remote enabling condition '{id_str}' status: {status:?}");
    }

    /// Register a fault monitor for specific enabling conditions.
    pub(crate) fn register_monitor(
        self: &Arc<Self>,
        condition_ids: Vec<String>,
        callback: Arc<dyn EnablingConditionCallback>,
    ) -> Result<FaultMonitor, EnablingConditionError> {
        let monitor_id = MONITOR_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

        let entry = MonitorEntry {
            id: monitor_id,
            condition_ids,
            callback,
        };

        self.monitors
            .write()
            .map_err(|e| {
                error!("monitors lock poisoned in register_monitor: {e}");
                EnablingConditionError::InternalError(format!("monitors lock poisoned: {e}"))
            })?
            .push(entry);

        debug!("Registered fault monitor #{monitor_id}");
        Ok(FaultMonitor {
            monitor_id,
            manager: Arc::clone(self),
        })
    }

    /// Unregister a fault monitor by ID.
    fn unregister_monitor(&self, monitor_id: u64) {
        match self.monitors.write() {
            Ok(mut monitors) => {
                monitors.retain(|m| m.id != monitor_id);
                debug!("Unregistered fault monitor #{monitor_id}");
            }
            Err(e) => {
                error!("monitors lock poisoned in unregister_monitor: {e}");
            }
        }
    }

    /// Get the current status of an enabling condition.
    pub(crate) fn get_status(&self, entity: &str) -> Option<EnablingConditionStatus> {
        self.conditions.read().ok()?.get(entity).copied()
    }

    /// Dispatch a status change to all monitors watching the given condition.
    ///
    /// Collects callbacks under read lock, then invokes them after releasing
    /// the lock to avoid potential deadlocks.
    fn dispatch_to_monitors(&self, id: &str, status: EnablingConditionStatus) {
        let callbacks: Vec<Arc<dyn EnablingConditionCallback>> = {
            let monitors = match self.monitors.read() {
                Ok(m) => m,
                Err(e) => {
                    error!("monitors lock poisoned in dispatch_to_monitors: {e}");
                    return;
                }
            };
            monitors
                .iter()
                .filter(|m| m.condition_ids.iter().any(|c| c == id))
                .map(|m| Arc::clone(&m.callback))
                .collect()
        };

        for callback in callbacks {
            if let Err(e) = catch_unwind(AssertUnwindSafe(|| {
                callback.on_condition_change(id, status);
            })) {
                error!("Callback panicked for condition '{id}': {e:?}");
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::arithmetic_side_effects
)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn make_manager() -> Arc<EnablingConditionManager> {
        let sink: Arc<dyn crate::FaultSinkApi> = Arc::new(crate::test_utils::RecordingSink::new());
        Arc::new(EnablingConditionManager::with_sink(Arc::downgrade(&sink)))
    }

    #[test]
    fn register_condition_succeeds() {
        let manager = make_manager();
        let ec = manager.register_condition("vehicle.speed.valid");
        assert!(ec.is_ok());
        assert_eq!(
            manager.get_status("vehicle.speed.valid"),
            Some(EnablingConditionStatus::Inactive)
        );
    }

    #[test]
    fn register_duplicate_returns_error() {
        let manager = make_manager();
        let _ = manager.register_condition("vehicle.speed.valid").unwrap();
        let result = manager.register_condition("vehicle.speed.valid");
        assert!(result.is_err());
    }

    #[test]
    fn report_status_updates_state() {
        let manager = make_manager();
        let ec = manager.register_condition("engine.running").unwrap();
        ec.report_status(EnablingConditionStatus::Active).unwrap();
        assert_eq!(
            manager.get_status("engine.running"),
            Some(EnablingConditionStatus::Active)
        );
    }

    #[test]
    fn report_status_skips_duplicate() {
        let manager = make_manager();
        let ec = manager.register_condition("engine.running").unwrap();
        ec.report_status(EnablingConditionStatus::Inactive).unwrap();
        // Same status again — should be a no-op
        ec.report_status(EnablingConditionStatus::Inactive).unwrap();
        assert_eq!(
            manager.get_status("engine.running"),
            Some(EnablingConditionStatus::Inactive)
        );
    }

    #[test]
    fn monitor_receives_callback_on_status_change() {
        let manager = make_manager();
        let ec = manager.register_condition("vehicle.speed.valid").unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        let _monitor = manager
            .register_monitor(
                vec!["vehicle.speed.valid".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        ec.report_status(EnablingConditionStatus::Active).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn monitor_not_called_for_unrelated_condition() {
        let manager = make_manager();
        let _ec1 = manager.register_condition("vehicle.speed.valid").unwrap();
        let ec2 = manager.register_condition("engine.running").unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        let _monitor = manager
            .register_monitor(
                vec!["vehicle.speed.valid".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        // Change engine.running — monitor watches vehicle.speed.valid only
        ec2.report_status(EnablingConditionStatus::Active).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn monitor_watches_multiple_conditions() {
        let manager = make_manager();
        let ec1 = manager.register_condition("vehicle.speed.valid").unwrap();
        let ec2 = manager.register_condition("engine.running").unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        let _monitor = manager
            .register_monitor(
                vec![
                    "vehicle.speed.valid".to_string(),
                    "engine.running".to_string(),
                ],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        ec1.report_status(EnablingConditionStatus::Active).unwrap();
        ec2.report_status(EnablingConditionStatus::Active).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn monitor_unregisters_on_drop() {
        let manager = make_manager();
        let ec = manager.register_condition("vehicle.speed.valid").unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        {
            let _monitor = manager
                .register_monitor(
                    vec!["vehicle.speed.valid".to_string()],
                    Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                        count_clone.fetch_add(1, Ordering::SeqCst);
                    }),
                )
                .unwrap();

            ec.report_status(EnablingConditionStatus::Active).unwrap();
            assert_eq!(call_count.load(Ordering::SeqCst), 1);
        }
        // Monitor dropped — should not receive further callbacks
        ec.report_status(EnablingConditionStatus::Inactive).unwrap();
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn handle_remote_notification_updates_state_and_dispatches() {
        let manager = make_manager();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&call_count);

        let _monitor = manager
            .register_monitor(
                vec!["remote.condition".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    count_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        let id = ShortString::try_from("remote.condition".as_bytes()).unwrap();
        manager.handle_remote_notification(&id, EnablingConditionStatus::Active);

        assert_eq!(
            manager.get_status("remote.condition"),
            Some(EnablingConditionStatus::Active)
        );
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn entity_too_long_returns_error() {
        let manager = make_manager();
        let long_entity = "a".repeat(100); // > 64 bytes
        let result = manager.register_condition(&long_entity);
        assert!(result.is_err());
    }

    #[test]
    fn callback_panic_does_not_break_other_monitors() {
        let manager = make_manager();
        let ec = manager.register_condition("vehicle.speed.valid").unwrap();

        let count_before = Arc::new(AtomicUsize::new(0));
        let count_after = Arc::new(AtomicUsize::new(0));
        let before_clone = Arc::clone(&count_before);
        let after_clone = Arc::clone(&count_after);

        // Monitor 1: increments counter
        let _m1 = manager
            .register_monitor(
                vec!["vehicle.speed.valid".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    before_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        // Monitor 2: panics
        let _m2 = manager
            .register_monitor(
                vec!["vehicle.speed.valid".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    panic!("deliberate test panic");
                }),
            )
            .unwrap();

        // Monitor 3: increments counter
        let _m3 = manager
            .register_monitor(
                vec!["vehicle.speed.valid".to_string()],
                Arc::new(move |_id: &str, _status: EnablingConditionStatus| {
                    after_clone.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();

        ec.report_status(EnablingConditionStatus::Active).unwrap();

        assert_eq!(
            count_before.load(Ordering::SeqCst),
            1,
            "Monitor before panic should receive callback"
        );
        assert_eq!(
            count_after.load(Ordering::SeqCst),
            1,
            "Monitor after panic should receive callback"
        );

        // Subsequent notifications still work
        ec.report_status(EnablingConditionStatus::Inactive).unwrap();
        assert_eq!(count_before.load(Ordering::SeqCst), 2);
        assert_eq!(count_after.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn set_sink_wires_after_construction() {
        let manager = Arc::new(EnablingConditionManager::new());
        let sink: Arc<dyn crate::FaultSinkApi> = Arc::new(crate::test_utils::RecordingSink::new());

        // Before wiring: register works (IPC send is best-effort)
        let ec = manager.register_condition("test.condition").unwrap();

        // Wire sink
        manager.set_sink(Arc::downgrade(&sink));

        // After wiring: report_status sends to sink without panic
        ec.report_status(EnablingConditionStatus::Active).unwrap();
        assert_eq!(
            manager.get_status("test.condition"),
            Some(EnablingConditionStatus::Active)
        );
    }
}
