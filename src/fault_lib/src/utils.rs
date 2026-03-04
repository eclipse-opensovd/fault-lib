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
// Re-export utility functions from common for backward compatibility.
pub use common::types::{to_static_long_string, to_static_short_string};

#[doc(hidden)]
#[macro_export]
macro_rules! __fault_descriptor_option {
    () => {
        None
    };
    ($value:expr) => {
        Some($value)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __fault_descriptor_optional_summary {
    () => {
        None
    };
    ($value:literal) => {{
        #[allow(clippy::expect_used)]
        {
            Some($crate::utils::to_static_long_string($value).expect(concat!(
                "fault_descriptor!: summary '",
                $value,
                "' exceeds LongString capacity"
            )))
        }
    }};
}

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
        #[allow(clippy::expect_used)]
        {
            common::fault::FaultDescriptor {
                id: $id,
                name: $crate::utils::to_static_short_string($name)
                    .expect(concat!("fault_descriptor!: name '", $name, "' exceeds ShortString capacity")),
                category: $kind,
                severity: $sev,
                compliance: $crate::__fault_descriptor_compliance_vec!($($($ctag),*)?),
                reporter_side_debounce: $crate::__fault_descriptor_option!($($debounce)?),
                reporter_side_reset: $crate::__fault_descriptor_option!($($reset)?),
                manager_side_debounce: None,
                manager_side_reset: None,
                summary: $crate::__fault_descriptor_optional_summary!($($summary)?),
            }
        }
    }};
}

// Note: to_static_short_string and to_static_long_string are now defined
// in common::types and re-exported above for backward compatibility.
