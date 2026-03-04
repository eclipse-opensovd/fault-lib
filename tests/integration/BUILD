# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# This program and the accompanying materials are made available under the
# terms of the Apache License Version 2.0 which is available at
# https://www.apache.org/licenses/LICENSE-2.0

load("@rules_rust//rust:defs.bzl", "rust_test")

filegroup(
    name = "integration_srcs",
    srcs = glob(["src/**/*.rs"]),
)

# E2E integration tests exercising the full fault-lib → DFM pipeline
# without iceoryx2 IPC (in-process wiring only).
rust_test(
    name = "tests",
    srcs = [":integration_srcs"],
    edition = "2024",
    deps = [
        "//src/common",
        "//src/dfm_lib",
        "//src/fault_lib",
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:serial_test",
        "@score_fault_lib_crates//:tempfile",
    ],
)
