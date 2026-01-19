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
use common::types::*;
use iceoryx2_bb_container::string::*;

macro_rules! __fault_descriptor_optional_str {
    () => {
        None
    };
    ($value:literal) => {
        Some(::std::borrow::Cow::Borrowed($value))
    };
}

//    pub id: FaultId,

//     pub name: ShortString,
//     pub summary: Option<LongString>,

//     pub category: FaultType,
//     pub severity: FaultSeverity,
//     pub compliance: ComplianceVec,

//     pub reporter_side_debounce: Option<DebounceMode>,
//     pub reporter_side_reset: Option<ResetPolicy>,
//     pub manager_side_debounce: Option<DebounceMode>,
//     pub manager_side_reset: Option<ResetPolicy>,
// }

#[doc(hidden)]
#[macro_export]
macro_rules! __fault_descriptor_compliance_vec {
    // No compliance tags => empty ComplianceVec
    () => {{
        let v: common::fault::ComplianceVec = common::fault::ComplianceVec::new();
        v
    }};
    // One or more tags => fill the ComplianceVec
    ($($ctag:expr),+ $(,)?) => {{
        let mut v: common::fault::ComplianceVec = common::fault::ComplianceVec::new();
        $(
            v.push($ctag);
        )+
        v
    }};
}

#[macro_export]
macro_rules! fault_descriptor {
    // Minimal form; policies can be added via builder functions if desired.
    (
        id = $id:expr,
        name = $name:literal,
        kind = $kind:expr,
        severity = $sev:expr
        $(, compliance = [$($ctag:expr),* $(,)?])?
        $(, summary = $summary:literal)?
        $(, debounce = $debounce:expr)?
        $(, reset = $reset:expr)?
    ) => {{
        common::fault::FaultDescriptor {
            id: $id,
            name: $name,
            category: $kind,
            severity: $sev,
            compliance: $crate::__fault_descriptor_compliance_vec!($($($ctag),*)?),
            reporter_side_debounce: $(Some($debounce))?,
            reporter_side_reset: $(Some($reset))?,
            manager_side_debounce : None ,
            manager_side_reset : None,
            summary: $crate::utils::__fault_descriptor_optional_str!($($summary)?),
        }
    }};
}

pub fn to_static_short_string<T: AsRef<[u8]>>(input: T) -> Result<ShortString, iceoryx2_bb_container::string::StringModificationError> {
    StaticString::try_from(input.as_ref())
}

pub fn to_static_long_string<T: AsRef<[u8]>>(input: T) -> Result<LongString, iceoryx2_bb_container::string::StringModificationError> {
    StaticString::try_from(input.as_ref())
}
