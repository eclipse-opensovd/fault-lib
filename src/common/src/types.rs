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

use crate::fault::FaultRecord;
use iceoryx2::prelude::ZeroCopySend;
use iceoryx2_bb_container::{string::StaticString, vector::StaticVec};

pub type ShortString = StaticString<64>;
pub type LongString = StaticString<128>;
pub type MetadataVec = StaticVec<(ShortString, ShortString), 8>;
pub type Sha256Vec = StaticVec<u8, 32>;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, ZeroCopySend)]
#[repr(C)]
pub enum DiagnosticEvent {
    Hash((LongString, Sha256Vec)),
    Fault((LongString, FaultRecord)),
}
