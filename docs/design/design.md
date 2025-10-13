# Fault Library Design

The high-level design of OpenSOVD can be found here: [OpenSOVD Design](https://github.com/eclipse-opensovd/opensovd/blob/main/docs/design/design.md)

Fault Library

- Provides a framework agnostic interface for apps or FEO activities to report faults - called "Fault API" in the S-CORE architecture.
- **The Fault lib is the interface between the S-CORE and the OpenSOVD project and should be developed in cooperation - see [ADR S-CORE Interface](https://github.com/eclipse-opensovd/opensovd/blob/main/docs/design/adr/001-adr-score-interface.md).**
- Relays faults via IPC to central Diagnostic Fault Manager.
- Enables domain-specific error logic (e.g. debouncing) by exposing a configuration interface
- Reporting of faults additionally results in a log entry.
- The interface needs to be specified further but will likely include:
- Fault ID (FID)
- time
- ENUM fault type (like DLT ENUMs)
- optional meta data
- Fault lib is the base for activity specific, custom fault handling.
- Can and should also be used by platform components to report faults.
- Potentially source of faults to be acted upon - e.g. by S-CORE Health and Lifecycle Management.
- Also needs to enforce regulatory requirements for certain faults - e.g. emission relevant.
- Decentral component.
