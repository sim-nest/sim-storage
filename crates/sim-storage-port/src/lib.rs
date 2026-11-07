// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Portable contracts between SIM's Table/Dir policy and host storage.
//!
//! Paths are represented only as validated relative components. Native paths,
//! handles, synchronization, and error translation belong to platform adapters.

mod cancel;
mod commit;
mod entry;
mod error;
mod exchange;
mod observation;
mod port;

pub use cancel::{Cancellation, NeverCancel};
pub use commit::CommitAction;
pub use entry::{HostEntry, HostEntryKind};
pub use error::{HostDirError, HostDirErrorKind, PortResult};
pub use exchange::HostCompareExchange;
pub use observation::HostDirObservation;
pub use port::HostDirPort;
