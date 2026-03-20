/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 */

use alloc::borrow::Cow;

/// Errors that may occur when publishing a fault record through a sink.
#[derive(thiserror::Error, Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum SinkError {
    /// The IPC transport is not available (e.g. DFM not running).
    #[error("transport unavailable")]
    TransportDown,
    /// The event was dropped because the publish rate exceeded a limit.
    #[error("rate limited")]
    RateLimited,
    /// The caller lacks permission to publish on this channel.
    #[error("permission denied")]
    PermissionDenied,
    /// The fault descriptor is invalid or refers to an unknown fault.
    #[error("invalid descriptor: {0}")]
    BadDescriptor(Cow<'static, str>),
    /// Catch-all for errors not covered by specific variants.
    #[error("other: {0}")]
    Other(Cow<'static, str>),
    /// The iceoryx2 service name could not be created.
    #[error("invalid service name")]
    InvalidServiceName,
    /// The operation timed out.
    #[error("timeout")]
    Timeout,
    /// The internal send queue is full.
    #[error("queue full")]
    QueueFull,
}
