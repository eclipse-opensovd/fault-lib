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
            debounce: None$(.or(Some($debounce)))?,
            reset: None$(.or(Some($reset)))?,
            summary: None$(.or(Some($summary)))?,
            docs_url: None$(.or(Some($docs)))?,
        }
    }};
}
