//! The [`OverrideTable`] object: an overlay of layered tables whose lookups
//! resolve front-to-back, implementing the kernel table, object-encoding, and
//! citizen contracts.

use std::collections::BTreeSet;

use sim_kernel::{
    Cx, Error, Expr, Object, ObjectEncode, ObjectEncoding, Result, Symbol, Table, Value,
    id::CORE_TABLE_CLASS_ID, object::ClassRef,
};

/// Overlay table that layers one or more tables and resolves lookups
/// front-to-back, so earlier layers shadow later ones.
///
/// Reads (`get`/`has`/`keys`/`entries`/`len`) consult layers in order and take
/// the first match, while writes (`set`/`del`/`clear`) target only the front
/// (first) layer. Each layer is itself a table [`Value`]; the override holds
/// references to them rather than copying their contents, so changes to the
/// underlying layers are visible through the overlay. Implements the kernel
/// [`Table`] contract along with the object-encoding and citizen contracts.
#[derive(Clone)]
pub struct OverrideTable {
    layers: Vec<Value>,
}

impl OverrideTable {
    /// Construct an override table over `layers`, ordered front (shadowing) to
    /// back (shadowed).
    ///
    /// # Errors
    ///
    /// Returns an error if `layers` is empty or if any layer value is not a
    /// table.
    pub fn new(layers: Vec<Value>) -> Result<Self> {
        if layers.is_empty() {
            return Err(Error::Eval(
                "table/override: expected at least one table layer".to_owned(),
            ));
        }
        for layer in &layers {
            if layer.object().as_table_impl().is_none() {
                return Err(Error::Eval(
                    "table/override: every layer must be a table".to_owned(),
                ));
            }
        }
        Ok(Self { layers })
    }

    /// The layer tables, ordered front (shadowing) to back (shadowed).
    pub fn layers(&self) -> &[Value] {
        &self.layers
    }

    fn front(&self) -> &Value {
        &self.layers[0]
    }
}

impl Object for OverrideTable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("table/override[layers={}]", self.layers.len()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for OverrideTable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = Symbol::qualified("core", "Table");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol)
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for OverrideTable {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        let args = self
            .layers
            .iter()
            .map(|layer| layer.object().as_expr(cx))
            .collect::<Result<Vec<_>>>()?;
        Ok(ObjectEncoding::Constructor {
            class: Symbol::new("OverrideTable"),
            args,
        })
    }
}

impl sim_citizen::Citizen for OverrideTable {
    fn citizen_symbol() -> Symbol {
        Symbol::new("OverrideTable")
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        1
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["layer"]
    }
}

impl Table for OverrideTable {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "override")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        for layer in &self.layers {
            let table = layer
                .object()
                .as_table_impl()
                .expect("validated table layer");
            if table.has(cx, key.clone())? {
                return table.get(cx, key);
            }
        }
        cx.factory().nil()
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        self.front()
            .object()
            .as_table_impl()
            .expect("validated table layer")
            .set(cx, key, value)
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        for layer in &self.layers {
            if layer
                .object()
                .as_table_impl()
                .expect("validated table layer")
                .has(cx, key.clone())?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        self.front()
            .object()
            .as_table_impl()
            .expect("validated table layer")
            .del(cx, key)
    }

    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for layer in &self.layers {
            for key in layer
                .object()
                .as_table_impl()
                .expect("validated table layer")
                .keys(cx)?
            {
                if seen.insert(key.clone()) {
                    out.push(key);
                }
            }
        }
        Ok(out)
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for layer in &self.layers {
            for (key, value) in layer
                .object()
                .as_table_impl()
                .expect("validated table layer")
                .entries(cx)?
            {
                if seen.insert(key.clone()) {
                    out.push((key, value));
                }
            }
        }
        Ok(out)
    }

    fn len(&self, cx: &mut Cx) -> Result<usize> {
        Ok(self.keys(cx)?.len())
    }

    fn clear(&self, cx: &mut Cx) -> Result<()> {
        self.front()
            .object()
            .as_table_impl()
            .expect("validated table layer")
            .clear(cx)
    }
}
