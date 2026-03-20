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

//! Enabling condition types for fault detection gating.
//!
//! Enabling conditions control whether fault monitors should actively
//! detect faults. When a condition is [`Active`](EnablingConditionStatus::Active),
//! the associated fault detection runs normally. When [`Inactive`](EnablingConditionStatus::Inactive),
//! fault detection is suspended.
//!
//! See `docs/puml/new_enable_condition.puml` and `docs/puml/enable_condition_ntf.puml`
//! for the design-level sequence diagrams.

use iceoryx2::prelude::ZeroCopySend;
use serde::{Deserialize, Serialize};

use crate::types::ShortString;

/// Status of an enabling condition.
///
/// Enabling conditions gate fault detection: when a condition is `Active`,
/// the associated fault monitors may report faults. When `Inactive`,
/// fault detection for dependent monitors is suspended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub enum EnablingConditionStatus {
    /// Condition is fulfilled — fault detection is enabled.
    Active,
    /// Condition is not fulfilled — fault detection is suspended.
    Inactive,
}

/// IPC notification broadcast from DFM to `FaultLib` instances when an
/// enabling condition status changes.
///
/// Published on the `dfm/enabling_condition/notification` IPC channel.
/// `FaultLib` subscribers use this to update local `FaultMonitor` state and
/// invoke registered callbacks.
#[derive(Debug, Clone, PartialEq, Eq, ZeroCopySend)]
#[repr(C)]
pub struct EnablingConditionNotification {
    /// Identifier of the enabling condition (matches the entity used during registration).
    pub id: ShortString,
    /// New status of the enabling condition.
    pub status: EnablingConditionStatus,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn status_copy_semantics() {
        let a = EnablingConditionStatus::Active;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn status_all_variants() {
        let statuses = [
            EnablingConditionStatus::Active,
            EnablingConditionStatus::Inactive,
        ];
        assert_eq!(statuses.len(), 2);
    }

    #[test]
    fn status_equality() {
        assert_eq!(
            EnablingConditionStatus::Active,
            EnablingConditionStatus::Active
        );
        assert_ne!(
            EnablingConditionStatus::Active,
            EnablingConditionStatus::Inactive
        );
    }

    #[test]
    fn notification_construction() {
        let id = ShortString::try_from("vehicle.speed".as_bytes()).unwrap();
        let ntf = EnablingConditionNotification {
            id,
            status: EnablingConditionStatus::Active,
        };
        assert_eq!(ntf.id.to_string(), "vehicle.speed");
        assert_eq!(ntf.status, EnablingConditionStatus::Active);
    }

    #[test]
    fn notification_clone() {
        let id = ShortString::try_from("engine.running".as_bytes()).unwrap();
        let ntf = EnablingConditionNotification {
            id,
            status: EnablingConditionStatus::Inactive,
        };
        let cloned = ntf.clone();
        assert_eq!(cloned.status, EnablingConditionStatus::Inactive);
    }
}
