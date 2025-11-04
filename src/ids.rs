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

use std::{borrow::Cow, fmt};

// Lightweight identifiers that keep fault attribution consistent across the fleet.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FaultId {
    Numeric(u32),                 // e.g., DTC-like
    Text(Cow<'static, str>),      // human-stable symbolic ID (runtime or static)
    Uuid([u8; 16]),               // global uniqueness if needed
}

impl FaultId {
    /// Convenience for constructing a textual ID from either a static string or owned `String`.
    pub fn text(value: impl Into<Cow<'static, str>>) -> Self {
        Self::Text(value.into())
    }

    /// `const` helper so descriptors can be defined in static contexts.
    pub const fn text_const(value: &'static str) -> Self {
        Self::Text(Cow::Borrowed(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId {
    pub entity: &'static str,         // e.g., "ADAS.Perception", "HVAC"
    pub ecu: Option<&'static str>,    // e.g., "ECU-A"
    pub domain: Option<&'static str>, // e.g., "ADAS", "IVI"
    pub sw_component: Option<&'static str>,
    pub instance: Option<&'static str>, // allow N instances
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ecu = self.ecu.unwrap_or("-");
        let dom = self.domain.unwrap_or("-");
        let comp = self.sw_component.unwrap_or("-");
        let inst = self.instance.unwrap_or("-");
        write!(
            f,
            "{}@ecu:{} dom:{} comp:{} inst:{}",
            self.entity, ecu, dom, comp, inst
        )
    }
}
