//! Loadable function exports for mounted Table/Dir namespaces.

use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Args, Callable, ClassRef, Cx, DefaultFactory, Dependency, Error, Export, Expr,
    Factory, Lib, LibManifest, LibTarget, Linker, LoadCx, Object, Result, Symbol, Value, Version,
};
use sim_table_core::TablePath;

use crate::{MountedDir, routing::inspection_expr, table_mount_capability};

/// Symbol for the namespace creation function.
pub fn mount_create_symbol() -> Symbol {
    Symbol::qualified("table/mount", "create")
}

/// Symbol for the directory-mount function.
pub fn mount_dir_symbol() -> Symbol {
    Symbol::qualified("table/mount", "dir")
}

/// Symbol for the table-mount function.
pub fn mount_table_symbol() -> Symbol {
    Symbol::qualified("table/mount", "table")
}

/// Symbol for the unmount function.
pub fn mount_unmount_symbol() -> Symbol {
    Symbol::qualified("table/mount", "unmount")
}

/// Symbol for the mount inspection function.
pub fn mount_inspect_symbol() -> Symbol {
    Symbol::qualified("table/mount", "inspect")
}

/// Loadable library for the mounted namespace function surface.
pub struct MountDirLib;

impl Lib for MountDirLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("table", "mount"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::<Dependency>::new(),
            capabilities: vec![table_mount_capability()],
            exports: function_symbols()
                .into_iter()
                .map(|symbol| Export::Function {
                    symbol,
                    function_id: None,
                })
                .collect(),
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for symbol in function_symbols() {
            linker.function_value(
                symbol.clone(),
                cx.factory().opaque(Arc::new(MountFunction { symbol }))?,
            )?;
        }
        Ok(())
    }
}

/// Install the mounted namespace library into `cx`.
pub fn install_mount_dir_lib(cx: &mut Cx) -> Result<()> {
    let lib_id = Symbol::qualified("table", "mount");
    if cx.registry().lib(&lib_id).is_some() {
        return Ok(());
    }
    cx.load_lib(&MountDirLib).map(|_| ())
}

fn function_symbols() -> Vec<Symbol> {
    vec![
        mount_create_symbol(),
        mount_dir_symbol(),
        mount_table_symbol(),
        mount_unmount_symbol(),
        mount_inspect_symbol(),
    ]
}

#[derive(Clone)]
struct MountFunction {
    symbol: Symbol,
}

impl Object for MountFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for MountFunction {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(class) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(class.clone());
        }
        DefaultFactory.class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(self.symbol.clone()))
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for MountFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let args = args.into_vec();
        match self.symbol.to_string().as_str() {
            "table/mount/create" => call_create(cx, args),
            "table/mount/dir" => call_mount_dir(cx, args),
            "table/mount/table" => call_mount_table(cx, args),
            "table/mount/unmount" => call_unmount(cx, args),
            "table/mount/inspect" => call_inspect(cx, args),
            _ => Err(Error::Eval(format!(
                "table/mount: unknown function {}",
                self.symbol
            ))),
        }
    }
}

fn call_create(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    cx.require(&table_mount_capability())?;
    let [root] = expect_arity(args, 1, "table/mount/create")?;
    cx.factory().opaque(Arc::new(MountedDir::new(root)?))
}

fn call_mount_dir(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let [namespace, path, target] = expect_arity(args, 3, "table/mount/dir")?;
    let dir = namespace_ref(&namespace)?;
    let path = path_arg(cx, &path)?;
    dir.mount_dir(cx, path, target)?;
    Ok(namespace)
}

fn call_mount_table(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let [namespace, path, target] = expect_arity(args, 3, "table/mount/table")?;
    let dir = namespace_ref(&namespace)?;
    let path = path_arg(cx, &path)?;
    dir.mount_table(cx, path, target)?;
    Ok(namespace)
}

fn call_unmount(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let [namespace, path] = expect_arity(args, 2, "table/mount/unmount")?;
    let dir = namespace_ref(&namespace)?;
    let path = path_arg(cx, &path)?;
    match dir.unmount(cx, &path)? {
        Some(value) => Ok(value),
        None => cx.factory().nil(),
    }
}

fn call_inspect(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let [namespace] = expect_arity(args, 1, "table/mount/inspect")?;
    let dir = namespace_ref(&namespace)?;
    cx.factory().expr(inspection_expr(&dir.inspect()?))
}

fn expect_arity<const N: usize>(
    args: Vec<Value>,
    expected: usize,
    function: &str,
) -> Result<[Value; N]> {
    if args.len() != expected {
        return Err(Error::Eval(format!(
            "{function} expects {expected} argument(s), got {}",
            args.len()
        )));
    }
    args.try_into()
        .map_err(|_| Error::Eval(format!("{function} internal arity mismatch")))
}

fn namespace_ref(value: &Value) -> Result<&MountedDir> {
    value
        .object()
        .downcast_ref::<MountedDir>()
        .ok_or_else(|| Error::Eval("table/mount: expected MountedDir namespace".to_owned()))
}

fn path_arg(cx: &mut Cx, value: &Value) -> Result<TablePath> {
    let expr = value.object().as_expr(cx)?;
    let text = match expr {
        Expr::String(text) => text,
        Expr::Symbol(symbol) => symbol.to_string(),
        _ => {
            return Err(Error::Eval(
                "table/mount: path argument must be a string or symbol".to_owned(),
            ));
        }
    };
    TablePath::parse_absolute(&text)
        .map_err(|err| Error::Eval(format!("table/mount: invalid mount path {text:?}: {err:?}")))
}
