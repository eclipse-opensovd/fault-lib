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
        $(, docs = $docs:literal)?
        $(, debounce = $debounce:expr)?
        $(, reset = $reset:expr)?
    ) => {{
        $crate::model::FaultDescriptor {
            id: $id,
            name: $name,
            fault_type: $kind,
            default_severity: $sev,
            compliance: &[$($($ctag),*,)?],
            debounce: $(Some($debounce))?,
            reset: $(Some($reset))?,
            summary: $(Some($summary))?,
            docs_url: $(Some($docs))?,
        }
    }};
}
