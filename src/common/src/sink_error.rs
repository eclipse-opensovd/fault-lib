// Copyright (c) 2025 Contributors to the Eclipse Foundation
//
// See the NOTICE file(s) distributed with this work for additional
// information regarding copyright ownership.
//
// This program and the accompanying materials are made available under the
// terms of the Apache License Version 2.0 which is available at
// <https://www.apache.org/licenses/LICENSE-2.0>
//
// SPDX-License-Identifier: Apache-2.0
//

#[derive(thiserror::Error, Debug, PartialEq, Eq, Copy, Clone)]
pub enum SinkError {
    #[error("transport unavailable")]
    TransportDown,
    #[error("rate limited")]
    RateLimited,
    #[error("permission denied")]
    PermissionDenied,
    #[error("invalid descriptor: {0}")]
    BadDescriptor(&'static str),
    #[error("other: {0}")]
    Other(&'static str),
    #[error("Invalid service name")]
    InvalidServiceName,
    #[error("Timeout")]
    Timeout,
}
