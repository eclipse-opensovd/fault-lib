use std::fmt;

// Lightweight identifiers that keep fault attribution consistent across the fleet.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FaultId {
    Numeric(u32),              // e.g., DTC-like
    Text(&'static str),        // human-stable symbolic ID
    Uuid([u8; 16]),            // global uniqueness if needed
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceId {
    pub entity: &'static str,          // e.g., "ADAS.Perception", "HVAC"
    pub ecu: Option<&'static str>,     // e.g., "ECU-A"
    pub domain: Option<&'static str>,  // e.g., "ADAS", "IVI"
    pub sw_component: Option<&'static str>,
    pub instance: Option<&'static str>,   // allow N instances
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ecu = self.ecu.unwrap_or("-");
        let dom = self.domain.unwrap_or("-");
        let comp = self.sw_component.unwrap_or("-");
        let inst = self.instance.unwrap_or("-");
        write!(f, "{}@ecu:{} dom:{} comp:{} inst:{}", self.entity, ecu, dom, comp, inst)
    }
}
