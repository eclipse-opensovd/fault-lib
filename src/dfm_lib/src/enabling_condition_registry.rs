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

//! DFM-side registry for enabling conditions.
//!
//! Tracks registered enabling conditions and their current statuses.
//! When a status changes, the registry notifies the communicator to
//! broadcast the change to all `FaultLib` subscribers.

use std::collections::HashMap;

use common::enabling_condition::EnablingConditionStatus;
use tracing::{debug, info, warn};

/// DFM-side registry of enabling conditions.
///
/// Thread-safe: the DFM communicator calls methods from its receiver thread.
/// No internal locking needed since the communicator is single-threaded.
pub struct EnablingConditionRegistry {
    /// Registered conditions: entity → current status.
    conditions: HashMap<String, EnablingConditionStatus>,
}

impl Default for EnablingConditionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl EnablingConditionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            conditions: HashMap::new(),
        }
    }

    /// Register a new enabling condition.
    ///
    /// If the condition is already registered, logs a warning and returns
    /// the current status. New conditions start as `Inactive`.
    pub fn register(&mut self, entity: &str) -> EnablingConditionStatus {
        if let Some(status) = self.conditions.get(entity) {
            warn!("Enabling condition '{entity}' already registered, current status: {status:?}");
            return *status;
        }
        let status = EnablingConditionStatus::Inactive;
        self.conditions.insert(entity.to_string(), status);
        info!("Registered enabling condition: {entity}");
        status
    }

    /// Update the status of an enabling condition.
    ///
    /// Returns `Some(new_status)` if the status actually changed (for
    /// notification dispatch), `None` if no change occurred.
    pub fn update_status(
        &mut self,
        entity: &str,
        status: EnablingConditionStatus,
    ) -> Option<EnablingConditionStatus> {
        if let Some(current) = self.conditions.get_mut(entity) {
            if *current == status {
                debug!("Enabling condition '{entity}' status unchanged: {status:?}");
                return None;
            }
            *current = status;
            info!("Enabling condition '{entity}' status changed to {status:?}");
            Some(status)
        } else {
            // Auto-register on first status report
            self.conditions.insert(entity.to_string(), status);
            info!("Auto-registered enabling condition '{entity}' with status {status:?}");
            Some(status)
        }
    }

    /// Get the current status of an enabling condition.
    #[must_use]
    pub fn get_status(&self, entity: &str) -> Option<EnablingConditionStatus> {
        self.conditions.get(entity).copied()
    }

    /// Get all registered conditions and their statuses.
    #[must_use]
    pub fn all_conditions(&self) -> &HashMap<String, EnablingConditionStatus> {
        &self.conditions
    }

    /// Number of registered enabling conditions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.conditions.len()
    }

    /// Whether there are no registered enabling conditions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_registry_is_empty() {
        let reg = EnablingConditionRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_creates_inactive_condition() {
        let mut reg = EnablingConditionRegistry::new();
        let status = reg.register("vehicle.speed.valid");
        assert_eq!(status, EnablingConditionStatus::Inactive);
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.get_status("vehicle.speed.valid"),
            Some(EnablingConditionStatus::Inactive)
        );
    }

    #[test]
    fn register_duplicate_returns_current_status() {
        let mut reg = EnablingConditionRegistry::new();
        reg.register("engine.running");
        reg.update_status("engine.running", EnablingConditionStatus::Active);
        let status = reg.register("engine.running");
        assert_eq!(status, EnablingConditionStatus::Active);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn update_status_returns_new_on_change() {
        let mut reg = EnablingConditionRegistry::new();
        reg.register("engine.running");
        let result = reg.update_status("engine.running", EnablingConditionStatus::Active);
        assert_eq!(result, Some(EnablingConditionStatus::Active));
    }

    #[test]
    fn update_status_returns_none_on_no_change() {
        let mut reg = EnablingConditionRegistry::new();
        reg.register("engine.running");
        let result = reg.update_status("engine.running", EnablingConditionStatus::Inactive);
        assert_eq!(result, None);
    }

    #[test]
    fn update_status_auto_registers_unknown_condition() {
        let mut reg = EnablingConditionRegistry::new();
        let result = reg.update_status("new.condition", EnablingConditionStatus::Active);
        assert_eq!(result, Some(EnablingConditionStatus::Active));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn get_status_returns_none_for_unknown() {
        let reg = EnablingConditionRegistry::new();
        assert_eq!(reg.get_status("nonexistent"), None);
    }

    #[test]
    fn all_conditions_returns_full_map() {
        let mut reg = EnablingConditionRegistry::new();
        reg.register("a");
        reg.register("b");
        reg.update_status("a", EnablingConditionStatus::Active);

        let all = reg.all_conditions();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("a"), Some(&EnablingConditionStatus::Active));
        assert_eq!(all.get("b"), Some(&EnablingConditionStatus::Inactive));
    }

    #[test]
    fn default_creates_empty_registry() {
        let reg = EnablingConditionRegistry::default();
        assert!(reg.is_empty());
    }
}
