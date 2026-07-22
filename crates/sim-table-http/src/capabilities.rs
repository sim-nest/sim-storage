//! Capability helpers for the HTTP table backend.

use sim_kernel::{CapabilityName, Cx, Error, Result};

/// The capability gating direct HTTP table operations (`net/http`).
pub fn table_http_capability() -> CapabilityName {
    CapabilityName::new("net/http")
}

/// Require direct HTTP authority, accepting compatibility aliases.
pub fn require_table_http(cx: &Cx) -> Result<()> {
    let canonical = table_http_capability();
    if cx.capabilities().contains(&canonical)
        || net_http_aliases()
            .iter()
            .any(|alias| cx.capabilities().contains(&CapabilityName::new(*alias)))
    {
        Ok(())
    } else {
        Err(Error::CapabilityDenied {
            capability: canonical,
        })
    }
}

fn net_http_aliases() -> &'static [&'static str] {
    &["net.http", "net-connect", "network"]
}
