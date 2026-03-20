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
use core::fmt;

use iceoryx2::prelude::ZeroCopySend;
use serde::{Deserialize, Serialize};

use crate::types::ShortString;

// Lightweight identifiers that keep fault attribution consistent across the fleet.

/// Identity of the component reporting a fault.
///
/// Encoded as a set of optional tags that together uniquely identify a
/// reporting entity within a vehicle.  All fields use IPC-safe fixed-size
/// strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroCopySend)]
#[repr(C)]
pub struct SourceId {
    /// Logical entity name (e.g. `"ADAS.Perception"`, `"HVAC"`).
    pub entity: ShortString,
    /// ECU identifier (e.g. `"ECU-A"`).
    pub ecu: Option<ShortString>,
    /// Domain grouping (e.g. `"ADAS"`, `"IVI"`).
    pub domain: Option<ShortString>,
    /// Software component name within the entity.
    pub sw_component: Option<ShortString>,
    /// Instance discriminator when multiple replicas exist.
    pub instance: Option<ShortString>,
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.entity)?;
        let tags: &[(&str, &Option<ShortString>)] = &[
            ("ecu", &self.ecu),
            ("dom", &self.domain),
            ("comp", &self.sw_component),
            ("inst", &self.instance),
        ];
        for (label, value) in tags {
            if let Some(v) = value {
                write!(f, " {label}:{v}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_short(s: &str) -> ShortString {
        ShortString::try_from(s.as_bytes()).unwrap()
    }

    fn full_source_id() -> SourceId {
        SourceId {
            entity: make_short("HVAC"),
            ecu: Some(make_short("ECU-A")),
            domain: Some(make_short("Climate")),
            sw_component: Some(make_short("Compressor")),
            instance: Some(make_short("0")),
        }
    }

    fn minimal_source_id() -> SourceId {
        SourceId {
            entity: make_short("ADAS"),
            ecu: None,
            domain: None,
            sw_component: None,
            instance: None,
        }
    }

    #[test]
    fn construction_all_fields() {
        let sid = full_source_id();
        assert_eq!(sid.entity.to_string(), "HVAC");
        assert_eq!(sid.ecu.unwrap().to_string(), "ECU-A");
        assert_eq!(sid.domain.unwrap().to_string(), "Climate");
        assert_eq!(sid.sw_component.unwrap().to_string(), "Compressor");
        assert_eq!(sid.instance.unwrap().to_string(), "0");
    }

    #[test]
    fn construction_entity_only() {
        let sid = minimal_source_id();
        assert_eq!(sid.entity.to_string(), "ADAS");
        assert!(sid.ecu.is_none());
        assert!(sid.domain.is_none());
        assert!(sid.sw_component.is_none());
        assert!(sid.instance.is_none());
    }

    #[test]
    fn display_all_fields_present() {
        let sid = full_source_id();
        let out = sid.to_string();
        assert_eq!(out, "HVAC ecu:ECU-A dom:Climate comp:Compressor inst:0");
    }

    #[test]
    fn display_entity_only_hides_none() {
        let sid = minimal_source_id();
        let out = sid.to_string();
        assert_eq!(out, "ADAS");
    }

    #[test]
    fn display_partial_fields() {
        let sid = SourceId {
            entity: make_short("IVI"),
            ecu: None,
            domain: Some(make_short("Infotainment")),
            sw_component: None,
            instance: Some(make_short("1")),
        };
        let out = sid.to_string();
        assert_eq!(out, "IVI dom:Infotainment inst:1");
    }

    #[test]
    fn partial_eq_identical() {
        let a = full_source_id();
        let b = full_source_id();
        assert_eq!(a, b);
    }

    #[test]
    fn partial_eq_different_entity() {
        let a = minimal_source_id();
        let mut b = minimal_source_id();
        b.entity = make_short("Different");
        assert_ne!(a, b);
    }

    #[test]
    fn clone_produces_equal_value() {
        let a = full_source_id();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn empty_string_entity() {
        let sid = SourceId {
            entity: ShortString::new(),
            ecu: None,
            domain: None,
            sw_component: None,
            instance: None,
        };
        assert_eq!(sid.to_string(), "");
    }

    #[test]
    fn hash_consistency() {
        use core::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        let a = full_source_id();
        let b = full_source_id();
        let mut ha = DefaultHasher::new();
        let mut hb = DefaultHasher::new();
        a.hash(&mut ha);
        b.hash(&mut hb);
        assert_eq!(ha.finish(), hb.finish());
    }

    #[test]
    fn serde_roundtrip_json() {
        let original = full_source_id();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: SourceId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }

    #[test]
    fn serde_roundtrip_minimal() {
        let original = minimal_source_id();
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: SourceId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, restored);
    }
}
