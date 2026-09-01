#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Loadable command surface for checked relational data.

use std::{fmt, sync::Arc};

use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result as KernelResult, Symbol, Value, Version,
};

mod command;

pub use command::*;

/// Loadable relation command library.
#[derive(Clone)]
pub struct RelationCommandLib {
    backend: Arc<dyn RelationCommands>,
}
impl RelationCommandLib {
    /// Creates a command library over a provider-composed backend.
    pub fn new(backend: Arc<dyn RelationCommands>) -> Self {
        Self { backend }
    }
}

impl Lib for RelationCommandLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "relation-cli"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: relation_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }
    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> KernelResult<()> {
        linker.function_value(
            relation_entrypoint_symbol(),
            cx.factory()
                .opaque(Arc::new(Entrypoint(self.backend.clone())))?,
        )?;
        Ok(())
    }
}

/// Exact bootloader handoff symbol.
pub fn relation_entrypoint_symbol() -> Symbol {
    Symbol::qualified("cli", "main/relation")
}

#[derive(Clone)]
struct Entrypoint(Arc<dyn RelationCommands>);
impl Object for Entrypoint {
    fn display(&self, _: &mut Cx) -> KernelResult<String> {
        Ok("cli/main/relation".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for Entrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for Entrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> KernelResult<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing relation envelope".into()))?;
        let argv = envelope_args(cx, envelope)?;
        let command = parse(&argv).map_err(|e| Error::Eval(e.to_string()))?;
        let output = self
            .0
            .execute(&command)
            .map_err(|e| Error::Eval(e.to_string()))?;
        print!("{output}");
        cx.factory().bool(true)
    }
}
fn envelope_args(cx: &mut Cx, envelope: &Value) -> KernelResult<Vec<String>> {
    let table = envelope
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval("relation CLI envelope is not a table".into()))?;
    let value = table.get(cx, Symbol::new("args"))?;
    let Expr::List(items) = value.object().as_expr(cx)? else {
        return Err(Error::TypeMismatch {
            expected: "argument list",
            found: "non-list",
        });
    };
    items
        .into_iter()
        .map(|item| {
            if let Expr::String(v) = item {
                Ok(v)
            } else {
                Err(Error::TypeMismatch {
                    expected: "string argument",
                    found: "non-string",
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
