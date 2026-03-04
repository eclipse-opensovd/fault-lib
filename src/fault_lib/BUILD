# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# This program and the accompanying materials are made available under the
# terms of the Apache License Version 2.0 which is available at
# https://www.apache.org/licenses/LICENSE-2.0

load("@rules_rust//rust:defs.bzl", "rust_binary", "rust_library", "rust_test")

filegroup(
    name = "fault_lib_srcs",
    srcs = glob(["src/**/*.rs"]),
)

rust_library(
    name = "fault_lib",
    srcs = [":fault_lib_srcs"],
    crate_name = "fault_lib",
    edition = "2024",
    visibility = ["//visibility:public"],
    deps = [
        "//src/common",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:sha2",
        "@score_fault_lib_crates//:thiserror",
    ],
)

# Library variant with test utilities exposed (for example binaries).
rust_library(
    name = "fault_lib_testutils",
    srcs = [":fault_lib_srcs"],
    crate_features = ["testutils"],
    crate_name = "fault_lib",
    edition = "2024",
    deps = [
        "//src/common",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:sha2",
        "@score_fault_lib_crates//:thiserror",
    ],
)

# IPC integration tests are guarded by `#[serial(ipc)]` from the serial_test
# crate, ensuring tests that use real iceoryx2 shared-memory resources never
# run concurrently.  No global RUST_TEST_THREADS=1 is needed.
rust_test(
    name = "tests",
    srcs = [":fault_lib_srcs"],
    data = glob(["tests/data/**"]),
    edition = "2024",
    env = {
        "CARGO_MANIFEST_DIR": "src/fault_lib",
    },
    deps = [
        "//src/common",
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:mockall",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:serial_test",
        "@score_fault_lib_crates//:sha2",
        "@score_fault_lib_crates//:thiserror",
    ],
)

rust_binary(
    name = "tst_app",
    srcs = ["examples/tst_app.rs"],
    edition = "2024",
    deps = [
        ":fault_lib",
        "//src/common",
        "@score_fault_lib_crates//:clap",
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
    ],
)

rust_binary(
    name = "catalog_and_reporter",
    srcs = ["examples/catalog_and_reporter.rs"],
    edition = "2024",
    deps = [
        ":fault_lib_testutils",
        "//src/common",
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:sha2",
    ],
)
