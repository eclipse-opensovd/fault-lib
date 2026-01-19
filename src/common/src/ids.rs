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
use crate::types::*;
use iceoryx2::prelude::ZeroCopySend;
use std::fmt;

// Lightweight identifiers that keep fault attribution consistent across the fleet.

#[derive(Debug, Clone, PartialEq, Eq, Hash, ZeroCopySend)]
#[repr(C)]
pub struct SourceId {
    pub entity: ShortString,         // e.g., "ADAS.Perception", "HVAC"
    pub ecu: Option<ShortString>,    // e.g., "ECU-A"
    pub domain: Option<ShortString>, // e.g., "ADAS", "IVI"
    pub sw_component: Option<ShortString>,
    pub instance: Option<ShortString>, // allow N instances
}

const DEFAULT_TAG: ShortString = ShortString::new();

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ecu = self.ecu.unwrap_or(DEFAULT_TAG);
        let dom = self.domain.unwrap_or(DEFAULT_TAG);
        let comp = self.sw_component.unwrap_or(DEFAULT_TAG);
        let inst = self.instance.unwrap_or(DEFAULT_TAG);
        write!(f, "{}@ecu:{} dom:{} comp:{} inst:{}", self.entity, ecu, dom, comp, inst)
    }
}
