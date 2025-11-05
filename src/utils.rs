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

// Small macro helpers that keep descriptor definitions tidy in user code.

#[doc(hidden)]
#[macro_export]
macro_rules! __fault_descriptor_optional_str {
    () => {
        None
    };
    ($value:literal) => {
        Some(::std::borrow::Cow::Borrowed($value))
    };
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
        $crate::model::FaultDescriptor {
            id: $id,
            name: ::std::borrow::Cow::Borrowed($name),
            fault_type: $kind,
            default_severity: $sev,
            compliance: ::std::borrow::Cow::Borrowed(&[$($($ctag),*,)?]),
            debounce: $(Some($debounce))?,
            reset: $(Some($reset))?,
            summary: $crate::__fault_descriptor_optional_str!($($summary)?),
        }
    }};
}
