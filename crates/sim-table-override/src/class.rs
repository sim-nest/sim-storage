//! The class object for override tables: the [`construct_override_table`]
//! constructor plus the backing class type implementing the kernel `Class`,
//! `Callable`, and `ReadConstructor` contracts.

use std::sync::Arc;

use sim_kernel::{
    Args, Callable, Class, ClassId, ClassRef, Cx, Object, ReadConstructor, ReadConstructorRef,
    Result, ShapeRef, Symbol, TableRef, Value, id::CORE_CLASS_CLASS_ID,
};

use crate::OverrideTable;

/// Construct an [`OverrideTable`] from `args` (the layers, front to back) and
/// wrap it as an opaque table object.
///
/// Shared by the class call path and the read-constructor path. Returns an
/// error if `args` is empty or any layer is not a table.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{Cx, DefaultFactory, Expr, NoopEvalPolicy, Symbol, Table};
/// use sim_table_override::construct_override_table;
///
/// let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
/// let shadow = cx.factory().string("front".to_owned()).unwrap();
/// let base = cx.factory().string("back".to_owned()).unwrap();
/// let front = cx.new_table(vec![(Symbol::new("k"), shadow.clone())]).unwrap();
/// let back = cx.new_table(vec![(Symbol::new("k"), base)]).unwrap();
///
/// let overlay = construct_override_table(&mut cx, vec![front, back]).unwrap();
/// let table = overlay.object().as_table_impl().unwrap();
/// // The front layer shadows the back layer.
/// assert_eq!(table.get(&mut cx, Symbol::new("k")).unwrap(), shadow);
/// // Deleting through the override hides lower layers until the key is set again.
/// table.del(&mut cx, Symbol::new("k")).unwrap();
/// let deleted = table.get(&mut cx, Symbol::new("k")).unwrap();
/// assert_eq!(
///     deleted.object().as_expr(&mut cx).unwrap(),
///     Expr::Nil
/// );
/// ```
pub fn construct_override_table(cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
    let table = OverrideTable::new(args)?;
    cx.factory().opaque(Arc::new(table))
}

#[derive(Clone)]
pub(crate) struct OverrideTableClass {
    id: ClassId,
}

impl OverrideTableClass {
    pub(crate) fn new(id: ClassId) -> Self {
        Self { id }
    }
}

impl Object for OverrideTableClass {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<class OverrideTable>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for OverrideTableClass {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Class"))
        {
            return Ok(value.clone());
        }
        cx.factory()
            .class_stub(CORE_CLASS_CLASS_ID, Symbol::qualified("core", "Class"))
    }
    fn as_expr(&self, _cx: &mut Cx) -> Result<sim_kernel::Expr> {
        Ok(sim_kernel::Expr::Symbol(Symbol::new("OverrideTable")))
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
    fn as_class(&self) -> Option<&dyn Class> {
        Some(self)
    }
}

impl Callable for OverrideTableClass {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        construct_override_table(cx, args.into_vec())
    }
}

impl Class for OverrideTableClass {
    fn id(&self) -> ClassId {
        self.id
    }

    fn symbol(&self) -> Symbol {
        Symbol::new("OverrideTable")
    }

    fn constructor_shape(&self, cx: &mut Cx) -> Result<ShapeRef> {
        cx.factory().nil()
    }

    fn instance_shape(&self, cx: &mut Cx) -> Result<ShapeRef> {
        cx.factory().nil()
    }

    fn read_constructor(&self, cx: &mut Cx) -> Result<Option<ReadConstructorRef>> {
        Ok(Some(
            cx.factory()
                .opaque(Arc::new(OverrideTableReadConstructor))?,
        ))
    }

    fn members(&self, cx: &mut Cx) -> Result<TableRef> {
        cx.factory().table(Vec::new())
    }
}

struct OverrideTableReadConstructor;

impl Object for OverrideTableReadConstructor {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<read-constructor OverrideTable>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for OverrideTableReadConstructor {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::id::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_read_constructor(&self) -> Option<&dyn ReadConstructor> {
        Some(self)
    }
}

impl ReadConstructor for OverrideTableReadConstructor {
    fn symbol(&self) -> Symbol {
        Symbol::new("OverrideTable")
    }

    fn args_shape(&self, cx: &mut Cx) -> Result<ShapeRef> {
        cx.factory().nil()
    }

    fn construct_read(&self, cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
        construct_override_table(cx, args)
    }
}
