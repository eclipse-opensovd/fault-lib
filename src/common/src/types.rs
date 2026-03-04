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

use crate::enabling_condition::EnablingConditionStatus;
use crate::fault::FaultRecord;
use iceoryx2::prelude::ZeroCopySend;
use iceoryx2_bb_container::string::{StaticString, StringModificationError};
use iceoryx2_bb_container::vector::StaticVec;

/// IPC-safe fixed-size string (64 bytes).
pub type ShortString = StaticString<64>;
/// Capacity of [`LongString`] — the const-generic bound passed to `StaticString`.
pub const LONG_STRING_CAPACITY: usize = 128;
/// IPC-safe fixed-size string (128 bytes).
pub type LongString = StaticString<LONG_STRING_CAPACITY>;
/// Fixed-capacity key-value metadata vector (max 8 entries).
pub type MetadataVec = StaticVec<(ShortString, ShortString), 8>;
/// Fixed-capacity vector for SHA-256 hash bytes (32 bytes).
pub type Sha256Vec = StaticVec<u8, 32>;

/// IPC envelope that carries different diagnostic event types over a
/// single iceoryx2 channel.
///
/// Size disparity: `Fault` (~1KB) vs `EnablingConditionRegister` (~64B).
/// Boxing the large variant is NOT possible because this type crosses IPC
/// boundaries via iceoryx2 shared memory, which requires `#[repr(C)]` layout
/// and `ZeroCopySend`. `Box<T>` is a heap pointer — not valid across processes.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub enum DiagnosticEvent {
    /// Catalog hash for handshake verification.
    Hash((LongString, Sha256Vec)),
    /// Fault record with its catalog identifier.
    Fault((LongString, FaultRecord)),
    /// Register a new enabling condition with the DFM.
    /// Payload: SOVD entity name used as the condition identifier.
    EnablingConditionRegister(ShortString),
    /// Report an enabling condition status change to the DFM.
    /// Payload: (condition id, new status).
    EnablingConditionStatusChange((ShortString, EnablingConditionStatus)),
}

/// Convert a byte-like value into a [`ShortString`] (64-byte static string).
///
/// # Errors
///
/// Returns [`StringModificationError`] if the input exceeds 64 bytes.
pub fn to_static_short_string<T: AsRef<[u8]>>(
    input: T,
) -> Result<ShortString, StringModificationError> {
    StaticString::try_from(input.as_ref())
}

/// Convert a byte-like value into a [`LongString`] (128-byte static string).
///
/// # Errors
///
/// Returns [`StringModificationError`] if the input exceeds 128 bytes.
pub fn to_static_long_string<T: AsRef<[u8]>>(
    input: T,
) -> Result<LongString, StringModificationError> {
    StaticString::try_from(input.as_ref())
}
