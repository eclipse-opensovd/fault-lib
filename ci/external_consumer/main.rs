// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 The Contributors to Eclipse OpenSOVD (see CONTRIBUTORS)

use dfm_lib::DfmQueryApi;
use fault_lib::{FaultApi, FaultSinkApi, reporter::ReporterApi};

fn accepts_query_api(_query: &dyn DfmQueryApi) {}
fn accepts_fault_sink(_sink: &dyn FaultSinkApi) {}
fn accepts_reporter(_reporter: &dyn ReporterApi) {}

fn assert_send_sync<T: Send + Sync>() {}

fn main() {
    let _ = accepts_query_api as fn(&dyn DfmQueryApi);
    let _ = accepts_fault_sink as fn(&dyn FaultSinkApi);
    let _ = accepts_reporter as fn(&dyn ReporterApi);
    assert_send_sync::<FaultApi>();
}