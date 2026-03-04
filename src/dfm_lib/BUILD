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
    name = "dfm_lib_srcs",
    srcs = glob(["src/**/*.rs"]),
)

rust_library(
    name = "dfm_lib",
    srcs = [":dfm_lib_srcs"],
    crate_name = "dfm_lib",
    edition = "2024",
    visibility = ["//visibility:public"],
    deps = [
        "//src/common",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:rust_kvs",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:thiserror",
    ],
)

rust_test(
    name = "tests",
    srcs = [":dfm_lib_srcs"],
    edition = "2024",
    deps = [
        "//src/common",
        "@score_fault_lib_crates//:iceoryx2",
        "@score_fault_lib_crates//:iceoryx2-bb-container",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:mockall",
        "@score_fault_lib_crates//:rust_kvs",
        "@score_fault_lib_crates//:serde_json",
        "@score_fault_lib_crates//:serial_test",
        "@score_fault_lib_crates//:thiserror",
    ],
)

rust_binary(
    name = "dfm",
    srcs = ["examples/dfm.rs"],
    edition = "2024",
    deps = [
        ":dfm_lib",
        "//src/common",
        "@score_fault_lib_crates//:env_logger",
        "@score_fault_lib_crates//:log",
        "@score_fault_lib_crates//:tempfile",
    ],
)

rust_binary(
    name = "sovd_fm",
    srcs = ["examples/sovd_fault_manager.rs"],
    edition = "2024",
    deps = [
        ":dfm_lib",
        "//src/common",
        "@score_fault_lib_crates//:tempfile",
    ],
)
