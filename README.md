# Diagnostic Fault Library

A Rust library for managing diagnostic fault reporting, processing, and querying
in Software-Defined Vehicles.  The library implements the **fault lifecycle**
defined by ISO 14229 (UDS) and exposes fault state through an
[OpenSOVD](https://covesa.github.io/OpenSOVD/)-compatible interface.

---

## 📂 Project Structure

```
fault-lib/
├── src/
│   ├── common/          # Shared types: FaultId, FaultRecord, FaultCatalog, debounce …
│   ├── fault_lib/       # Reporter-side API (fault reporting, enabling conditions)
│   ├── dfm_lib/         # Diagnostic Fault Manager (processing, SOVD, KVS storage)
│   └── xtask/           # Developer automation (cargo xtask …)
├── tests/
│   └── integration/     # Integration tests (fault-lib ↔ DFM end-to-end)
├── examples/            # Runnable examples (DFM, SOVD fault manager, tst_app)
├── docs/                # Sphinx / mdBook documentation
│   └── design/          # Architecture & design decisions
├── .github/workflows/   # CI/CD pipelines
├── .vscode/             # Recommended VS Code settings
├── BUILD                # Root Bazel targets
├── MODULE.bazel         # Bazel module dependencies (bzlmod)
├── Cargo.toml           # Rust workspace root
├── project_config.bzl   # Project metadata for Bazel macros
├── LICENSE              # Apache-2.0
└── README.md            # This file
```

### Source Crates

| Crate | Description |
|-------|-------------|
| `common` | Foundational types shared by reporters and DFM: `FaultId`, `FaultRecord`, `FaultCatalog`, `DebounceMode`, compliance tags, IPC timestamps. |
| `fault_lib` | **Reporter-side API.** Applications use `Reporter` to publish fault records to DFM via iceoryx2 IPC. Includes enabling-condition guards and builder-pattern catalog configuration. |
| `dfm_lib` | **Diagnostic Fault Manager.** Receives records via `FaultRecordProcessor`, manages lifecycle state, persists to KVS (`KvsSovdFaultStateStorage`), and exposes faults through `SovdFaultManager`. |
| `xtask` | Developer automation tasks (e.g., code generation helpers). |

---

## 🚀 Getting Started

### 1️⃣ Clone the Repository

```sh
git clone https://github.com/eclipse-opensovd/fault-lib.git
cd fault-lib
```

### 2️⃣ Build

```sh
# Cargo (all crates)
cargo build --workspace

# Bazel (all targets)
bazel build //src/...
```

### 3️⃣ Run Examples

Start the Diagnostic Fault Manager (DFM):

```sh
cargo run -p dfm_lib --example dfm
```

The `dfm` process uses hardcoded fault catalogs matching the JSON files in
`src/fault_lib/tests/data/`.  When ready you will see:

```
[INFO  dfm_lib::fault_lib_communicator] FaultLibCommunicator listening...
```

In a **separate terminal**, start a reporter application:

```sh
cargo run -p fault_lib --features testutils --example tst_app -- -c src/fault_lib/tests/data/hvac_fault_catalog.json
# or
cargo run -p fault_lib --features testutils --example tst_app -- -c src/fault_lib/tests/data/ivi_fault_catalog.json
```

The `tst_app` reporter loops 20 times over every fault in the catalog,
alternating between `Failed` and `Passed` with a 200 ms delay.

### 4️⃣ Run Tests

```sh
# All workspace tests (unit + integration)
cargo test --workspace

# Integration tests only
cargo test -p integration_tests

# Bazel tests
bazel test //...
```

#### Integration Tests

The `tests/integration/` crate contains a comprehensive end-to-end test suite
exercising the full fault-lib - DFM - SOVD pipeline **without IPC**:

| Module | What it covers |
|--------|----------------|
| `test_report_and_query` | Basic report - process - SOVD query flow |
| `test_lifecycle_transitions` | Full lifecycle: NotTested - PreFailed - Failed - Passed |
| `test_persistent_storage` | KVS persistence across DFM restart, delete operations |
| `test_multi_catalog` | Multi-tenant catalog isolation, cross-catalog independence |
| `test_debounce_aging_cycles_ec` | Debounce, aging policies, operation cycles, enabling conditions |
| `test_error_paths` | Error handling, validation, edge cases |
| `test_boundary_values` | Boundary conditions, limits, overflow scenarios |
| `test_concurrent_access` | Thread safety, concurrent publish/query |
| `test_ipc_query` | IPC-based SOVD query/clear protocol |
| `test_json_catalog` | JSON catalog parsing, validation |

---

## 🛠 Quality Gates

CI automatically runs these checks on every PR and push to `main`.
To run them locally before pushing:

### Format check

```sh
cargo fmt --all -- --check
bazel test //:format.check
```

### Lint

```sh
cargo clippy --workspace --all-targets -- \
  -D warnings -D clippy::unwrap_used -D clippy::expect_used \
  -D clippy::todo -D clippy::unimplemented -A clippy::new_without_default
```

### Build & test

```sh
cargo build --workspace
cargo test --workspace
bazel build //src/...
bazel test //...
```

### Miri (Undefined Behavior)

```sh
cargo +nightly miri test --workspace
```

### Copyright headers

```sh
bazel test //:copyright.check
```

---

## 📖 Documentation

| Document | Description |
|----------|-------------|
| [Design](docs/design/design.md) | Architecture and design decisions |

To run a live preview of the documentation locally:

```sh
bazel run //docs:live_preview
```

---

## ⚙️ `project_config.bzl`

Project-specific metadata used by Bazel macros (e.g., `dash_license_checker`):

```python
PROJECT_CONFIG = {
    "asil_level": "QM",
    "source_code": ["rust"],
}
```
