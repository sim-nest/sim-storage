// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Compare-exchange observation.

/// Outcome of a host-byte compare-exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCompareExchange {
    /// Whether the replacement was published.
    pub exchanged: bool,
    /// Bytes observed at the linearization point, or `None` when absent.
    pub observed: Option<Vec<u8>>,
}
