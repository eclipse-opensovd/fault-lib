# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
#
# See the NOTICE file(s) distributed with this work for additional
# information regarding copyright ownership.
#
# This program and the accompanying materials are made available under the
# terms of the Apache License Version 2.0 which is available at
# https://www.apache.org/licenses/LICENSE-2.0

load("@rules_rust//rust:defs.bzl", "rust_library", "rust_test")

filegroup(
    name = "common_srcs",
    srcs = glob(["src/**/*.rs"]),
)

rust_library(
    name = "common",
    srcs = [":common_srcs"],
    crate_name = "common",
    edition = "2024",
    visibility = ["//visibility:public"],
    deps = [
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:sha2",
        "@score_fault_lib_crates//:thiserror",
    ],
)

rust_test(
    name = "tests",
    srcs = [":common_srcs"],
    edition = "2024",
    deps = [
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:mockall",
        "@score_fault_lib_crates//:serde",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:sha2",
        "@score_fault_lib_crates//:thiserror",
    ],
)
