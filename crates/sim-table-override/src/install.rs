//! Library registration for the override table backend: the [`Lib`] manifest,
//! class and citizen wiring, and the [`install_override_table_lib`] entry
//! point.

use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Cx, DefaultFactory, Dependency, Export, Factory, Lib, LibManifest, LibTarget,
    Linker, LoadCx, Result, Symbol, Version,
};

use crate::{OverrideTable, class::OverrideTableClass};

struct OverrideTableLib;

impl Lib for OverrideTableLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("table", "override"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::<Dependency>::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Class {
                symbol: Symbol::new("OverrideTable"),
                class_id: None,
            }],
        }
    }

    fn load(&self, _cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        register_override_table_class(linker)
    }
}

/// Load the override table library into `cx`, registering the `OverrideTable`
/// class and its read constructor.
///
/// Idempotent: if the `table/override` library is already present the call is a
/// no-op. After installation `OverrideTable` can be called as a class to build
/// overlays.
pub fn install_override_table_lib(cx: &mut sim_kernel::Cx) -> Result<()> {
    let lib_id = Symbol::qualified("table", "override");
    if cx.registry().lib(&lib_id).is_some() {
        return Ok(());
    }
    cx.load_lib(&OverrideTableLib).map(|_| ())
}

fn register_override_table_class(linker: &mut Linker<'_>) -> Result<()> {
    let symbol = Symbol::new("OverrideTable");
    let class_id = linker.class(symbol.clone())?;
    let class = Arc::new(OverrideTableClass::new(class_id));
    let value = DefaultFactory.opaque(class)?;
    linker.bind_class_value(class_id, value)
}

fn install_override_table_citizen(linker: &mut Linker<'_>) -> Result<()> {
    register_override_table_class(linker)
}

fn conformance_override_table_citizen(cx: &mut Cx) -> Result<()> {
    let layer = cx.factory().table(vec![(
        Symbol::new("answer"),
        cx.factory().string("front".to_owned())?,
    )])?;
    let table = OverrideTable::new(vec![layer])?;
    let value = cx.factory().opaque(Arc::new(table))?;
    sim_citizen::check_value_fixture(cx, value)
}

sim_citizen::inventory::submit! {
    sim_citizen::CitizenInfo {
        symbol: "OverrideTable",
        version: 0,
        crate_name: env!("CARGO_PKG_NAME"),
        arity: 1,
        install: install_override_table_citizen,
        conformance: conformance_override_table_citizen,
    }
}
