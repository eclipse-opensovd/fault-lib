use crate::{
    catalog::FaultCatalog,
    config::{ReportOptions, ReporterConfig},
    model::{FaultDescriptor, FaultRecord},
    sink::{FaultSink, LogHook},
};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone)]
// FaultApi is the long-lived handle that wires a sink and logger together.
pub struct FaultApi {
    sink: Arc<dyn FaultSink>,
    logger: Arc<dyn LogHook>,
    catalog: Arc<FaultCatalog>,
}

impl FaultApi {
    // Callers construct this once at bootstrap and share it across tasks.
    pub fn new(
        sink: Arc<dyn FaultSink>,
        logger: Arc<dyn LogHook>,
        catalog: Arc<FaultCatalog>,
    ) -> Self {
        Self {
            sink,
            logger,
            catalog,
        }
    }

    /// Expose the catalog backing this API so callers can export hashes, etc.
    pub fn catalog(&self) -> &FaultCatalog {
        &self.catalog
    }

    /// Create a lightweight reporter bound to a source & lifecycle phase.
    pub fn reporter(&self, cfg: ReporterConfig) -> Reporter {
        Reporter {
            api: self.clone(),
            cfg,
        }
    }
}

/// What callers hold and clone in their components.
#[derive(Clone)]
// Reporter carries the static config for a particular component or ECU.
pub struct Reporter {
    api: FaultApi,
    cfg: ReporterConfig,
}

impl Reporter {
    pub fn cfg(&self) -> &ReporterConfig {
        &self.cfg
    }

    pub fn api(&self) -> &FaultApi {
        &self.api
    }
    /// Report an occurrence of a fault. Always logs via LogHook, then publishes via sink.
    #[allow(async_fn_in_trait)]
    pub fn report(
        &self,
        record: &FaultRecord,
        // descriptor: &FaultDescriptor,
        // opts: ReportOptions,
    ) -> Result<(), crate::sink::SinkError> {
        // let ReportOptions {
        //     severity,
        //     metadata,
        //     debounce,
        //     reset,
        //     extra_compliance,
        // } = opts;

        // let effective_severity = severity.unwrap_or(descriptor.default_severity);
        // let effective_debounce = debounce.or_else(|| descriptor.debounce.clone());
        // let effective_reset = reset.or_else(|| descriptor.reset.clone());

        // let mut metadata_out = self.cfg.default_meta.clone();
        // metadata_out.extend(metadata);

        // let mut compliance = descriptor.compliance.to_vec();
        // compliance.extend(extra_compliance);

        // let record = FaultRecord {
        //     time: SystemTime::now(),
        //     descriptor: descriptor.clone(),
        //     severity: effective_severity,
        //     source: self.cfg.source.clone(),
        //     lifecycle_phase: self.cfg.lifecycle_phase,
        //     metadata: metadata_out,
        //     catalog_id: self.api.catalog.id,
        //     catalog_version: self.api.catalog.version,
        //     compliance,
        //     effective_debounce,
        //     effective_reset,
        // };

        // 1) Local log.
        self.api.logger.on_report(&record);

        // 2) Ship the record; the sink decides buffering/retry policy.
        self.api.sink.publish(&record)
    }
}
