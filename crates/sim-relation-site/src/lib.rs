//! Bounded, provider-neutral relation effects.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod contract;
mod site;

pub use contract::*;
pub use site::RelationSite;
#[cfg(test)]
pub(crate) use site::{BoundedSink, Counts, enforce_work, kernel};

use std::sync::Arc;

use sim_kernel::{
    AbiVersion, ClassId, Cx, Export, Factory, Lib, LibManifest, LibTarget, Linker,
    Result as KernelResult, Symbol, Version,
};

/// Loadable relation-site library.
pub struct RelationSiteLib {
    site: RelationSite,
}
impl RelationSiteLib {
    /// Creates a library exporting `site`.
    pub fn new(site: RelationSite) -> Self {
        Self { site }
    }
}
impl Lib for RelationSiteLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("relation", "site-lib"),
            version: Version(env!("CARGO_PKG_VERSION").into()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Site {
                symbol: self.site.placement.site.clone(),
                runtime_id: None,
            }],
        }
    }
    fn load(&self, _: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> KernelResult<()> {
        linker.site_value(
            self.site.placement.site.clone(),
            sim_kernel::DefaultFactory.opaque(Arc::new(RelationSite::new(
                self.site.placement.clone(),
                self.site.driver.clone(),
            )))?,
        )?;
        Ok(())
    }
}

impl sim_kernel::Object for RelationSite {
    fn display(&self, _: &mut Cx) -> KernelResult<String> {
        Ok(format!("#<relation-site {}>", self.placement.site))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for RelationSite {
    fn class(&self, cx: &mut Cx) -> KernelResult<sim_kernel::ClassRef> {
        cx.factory()
            .class_stub(ClassId(0), Symbol::qualified("relation", "Site"))
    }
}

#[cfg(test)]
mod sqlite_locator_tests;
#[cfg(test)]
mod tests;
