
# Diagnostic Fault Library

This repository contains Diagnostic Fault library

---

## 📂 Project Structure

| File/Folder                         | Description                                       |
| ----------------------------------- | ------------------------------------------------- |
| `README.md`                         | Short description & build instructions            |
| `src/`                              | Source files for the module                       |
| `tests/`                            | Unit tests (UT) and integration tests (IT)        |
| `examples/`                         | Example files used for guidance                   |
| `docs/`                             | Documentation (Doxygen for C++ / mdBook for Rust) |
| `.github/workflows/`                | CI/CD pipelines                                   |
| `.vscode/`                          | Recommended VS Code settings                      |
| `.bazelrc`, `MODULE.bazel`, `BUILD` | Bazel configuration & settings                    |
| `project_config.bzl`                | Project-specific metadata for Bazel macros        |
| `LICENSE.md`                        | Licensing information                             |
| `CONTRIBUTION.md`                   | Contribution guidelines                           |

---

## 🚀 Getting Started

### 1️⃣ Clone the Repository

```sh
git clone https://github.com/eclipse-score/inc_diag.git
cd inc_diag
```

### 2️⃣ Build the Examples of module

To build the example showing how to use the fault-lib use:

```sh
bazel build //src/fault-lib:fault-lib-basic-ex
```

or directly run it with:

```sh
bazel run //src/fault-lib:fault-lib-basic-ex
```

To build all targets of the module the following command can be used:

```sh
bazel build //src/...
```

This command will instruct Bazel to build all targets that are under Bazel
package `src/`. The ideal solution is to provide single target that builds
artifacts, for example:

```sh
bazel build //src/<module_name>:release_artifacts
```

where `:release_artifacts` is filegroup target that collects all release
artifacts of the module.

> NOTE: This is just proposal, the final decision is on module maintainer how
> the module code needs to be built.



### 3️⃣ Run Tests

To get the fault lib example with the test instance of the diagnostic fault manager side running do the following steps.
Start the test version of the Diagnostic Fault manager:
```sh
cargo run --example=dfm
```

The `dfm` process uses hardcoded fault catalogs (only to show the possibility), which are equal to the example fault catalog json files stored under `src/fault_lib/tests/data/`.
When the `dfm` is running you should see the following log message 
```sh
[2026-01-04T20:46:11Z INFO  dfm_lib::fault_lib_communicator] FaultLibCommunicator listening...
```

Now the reporting application can be started. For that call in new console:

```sh
cargo run --bin tst_app -- -c src/fault_lib/tests/data/ivi_fault_catalog.json
```

and / or 

```sh
cargo run --bin tst_app -- -c src/fault_lib/tests/data/hvac_fault_catalog.json
```

The `tst_app` process reads the fault catalog and loops 20 times over all the fault's IDs present in the catalog. 
In each loop the `tst_app` uses `fault-lib` API and reports the faults either to be pass or failed with small delay (200ms) between loops. 

You should be able to see the `dfm` process reporting the faults to be received and stored, e.g.:

```
[2026-01-04T20:48:37Z INFO  dfm_lib::fault_lib_communicator] Received new fault ID: Numeric(28673)
[2026-01-04T20:48:37Z INFO  dfm_lib::fault_record_processor] Fault ID Numeric(28673) stored : true
[2026-01-04T20:48:37Z INFO  dfm_lib::fault_lib_communicator] Received new fault ID: Text(StaticString<64> { len: 33, data: "hvac.blower.speed_sensor_mismatch" })
[2026-01-04T20:48:37Z INFO  dfm_lib::fault_record_processor] Fault ID Text(StaticString<64> { len: 33, data: "hvac.blower.speed_sensor_mismatch" }) stored : true
[2026-01-04T20:48:38Z INFO  dfm_lib::fault_lib_communicator] Received new fault ID: Text(StaticString<64> { len: 2, data: "d1" })
[2026-01-04T20:48:38Z INFO  dfm_lib::fault_record_processor] Fault ID Text(StaticString<64> { len: 2, data: "d1" }) stored : true
[2026-01-04T20:48:38Z INFO  dfm_lib::fault_lib_communicator] Received new fault ID: Text(StaticString<64> { len: 2, data: "d2" })
[2026-01-04T20:48:38Z INFO  dfm_lib::fault_record_processor] Fault ID Text(StaticString<64> { len: 2, data: "d2" }) stored : true
[2026-01-04T20:48:38Z INFO  dfm_lib::fault_lib_communicator] Received new fault ID: Numeric(28673)
```


---

## 🛠 Tools & Linters

The template integrates **tools and linters** from **centralized repositories** to ensure consistency across projects.

- **C++:** `clang-tidy`, `cppcheck`, `Google Test`
- **Rust:** `clippy`, `rustfmt`, `Rust Unit Tests`
- **CI/CD:** GitHub Actions for automated builds and tests

---

## 📖 Documentation

To run localy the live preview of the documentation:

```sh
bazel run //docs:live_preview
```


---

## ⚙️ `project_config.bzl`

This file defines project-specific metadata used by Bazel macros, such as `dash_license_checker`.

### 📌 Purpose

It provides structured configuration that helps determine behavior such as:

- Source language type (used to determine license check file format)
- Safety level or other compliance info (e.g. ASIL level)

### 📄 Example Content

```python
PROJECT_CONFIG = {
    "asil_level": "QM",  # or "ASIL-A", "ASIL-B", etc.
    "source_code": ["cpp", "rust"]  # Languages used in the module
}
```

### 🔧 Use Case

When used with macros like `dash_license_checker`, it allows dynamic selection of file types
 (e.g., `cargo`, `requirements`) based on the languages declared in `source_code`.
