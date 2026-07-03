//! The [`ConsList`] object: a singly linked, shared cons-cell list that
//! implements the kernel list value and object-encoding contracts.

use std::{cmp::Ordering, sync::Arc};

use sim_kernel::{
    CORE_LIST_CLASS_ID, ClassRef, Cx, Expr, LengthResult, ListValue, Object, ObjectEncode,
    ObjectEncoding, Result, Symbol, Value, force_list_to_vec,
};

use crate::citizen::cons_list_class_symbol;

/// A singly linked, shared cons-cell list.
///
/// Each node holds an optional `car` (the head value) and an optional `cdr`
/// (a shared reference to the next node). The unique empty list is the node
/// with both fields `None`; a non-empty list is a chain of [`ConsList::cell`]
/// nodes terminated by an empty node. Nodes are shared through [`Arc`], so a
/// [`cdr`](ListValue::cdr) is a cheap pointer clone and tails can be shared
/// across many lists.
///
/// `ConsList` is the concrete object behind the `cons`
/// [`ListBackend`](sim_kernel::ListBackend) ([`crate::ConsBackend`]); it
/// implements the kernel list and
/// object-encoding contracts so the runtime treats it as a first-class list
/// value.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, ListValue, LengthResult};
/// use sim_list_cell::ConsList;
///
/// let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
///
/// // The empty list.
/// let empty = ConsList::from_vec(vec![]);
/// assert!(empty.is_empty(&mut cx).unwrap());
/// assert_eq!(empty.len(&mut cx).unwrap(), LengthResult::Known(0));
///
/// // A two-element list, read by head and tail.
/// let a = cx.factory().bool(true).unwrap();
/// let b = cx.factory().bool(false).unwrap();
/// let xs = ConsList::from_vec(vec![a, b]);
/// assert_eq!(xs.len(&mut cx).unwrap(), LengthResult::Known(2));
/// assert!(xs.car(&mut cx).unwrap().is_some());
/// let tail = xs.cdr(&mut cx).unwrap().unwrap();
/// let tail = tail.object().as_list().unwrap();
/// assert_eq!(tail.len(&mut cx).unwrap(), LengthResult::Known(1));
/// ```
#[derive(Clone)]
pub struct ConsList {
    /// The head value of this node, or `None` for the empty list.
    car: Option<Value>,
    /// The shared remainder of the list, or `None` for the empty list.
    cdr: Option<Arc<ConsList>>,
}

impl ConsList {
    /// Returns the empty list (a node with no head and no tail).
    pub fn empty() -> Self {
        Self {
            car: None,
            cdr: None,
        }
    }

    /// Builds a non-empty cell prepending `car` onto the shared tail `cdr`.
    pub fn cell(car: Value, cdr: Arc<ConsList>) -> Self {
        Self {
            car: Some(car),
            cdr: Some(cdr),
        }
    }

    /// Builds a shared list from `items` in order, the first item at the head.
    pub fn from_vec(items: Vec<Value>) -> Arc<Self> {
        let mut acc = Arc::new(Self::empty());
        for item in items.into_iter().rev() {
            acc = Arc::new(Self::cell(item, acc));
        }
        acc
    }

    fn count_cells(&self) -> usize {
        let mut count = 0usize;
        let mut cursor = self.cdr.as_ref().cloned();
        if self.car.is_none() {
            return 0;
        }

        count += 1;
        while let Some(node) = cursor {
            if node.car.is_none() {
                break;
            }
            count += 1;
            cursor = node.cdr.as_ref().cloned();
        }
        count
    }

    fn len_cmp_cells(&self, n: usize) -> Ordering {
        if self.car.is_none() {
            return 0usize.cmp(&n);
        }

        let mut count = 1usize;
        if count > n {
            return Ordering::Greater;
        }

        let mut cursor = self.cdr.as_ref().cloned();
        while let Some(next) = cursor {
            if next.car.is_none() {
                break;
            }
            count += 1;
            if count > n {
                return Ordering::Greater;
            }
            cursor = next.cdr.as_ref().cloned();
        }
        count.cmp(&n)
    }
}

impl Object for ConsList {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("cons[...]".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for ConsList {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = cons_list_class_symbol();
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        let symbol = Symbol::qualified("core", "List");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_LIST_CLASS_ID, symbol)
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::List(
            force_list_to_vec(cx, self, "list as_expr")?
                .into_iter()
                .map(|value| value.object().as_expr(cx))
                .collect::<Result<Vec<_>>>()?,
        ))
    }
    fn truth(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(self.car.is_some())
    }
    fn as_list(&self) -> Option<&dyn ListValue> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for ConsList {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        let items = force_list_to_vec(cx, self, "list/ConsList citizen")?
            .into_iter()
            .map(|value| value.object().as_expr(cx))
            .collect::<Result<Vec<_>>>()?;
        Ok(ObjectEncoding::Constructor {
            class: cons_list_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                crate::citizen::expr_items::encode(&items),
            ],
        })
    }
}

impl sim_citizen::Citizen for ConsList {
    fn citizen_symbol() -> Symbol {
        cons_list_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        1
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["items"]
    }
}

impl ListValue for ConsList {
    fn is_empty(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(self.car.is_none())
    }

    fn car(&self, _cx: &mut Cx) -> Result<Option<Value>> {
        Ok(self.car.clone())
    }

    fn cdr(&self, cx: &mut Cx) -> Result<Option<Value>> {
        match &self.cdr {
            Some(next) => Ok(Some(cx.factory().opaque(next.clone())?)),
            None => Ok(None),
        }
    }

    fn len(&self, _cx: &mut Cx) -> Result<LengthResult> {
        Ok(LengthResult::Known(self.count_cells()))
    }

    fn len_cmp(&self, _cx: &mut Cx, n: usize) -> Result<Ordering> {
        Ok(self.len_cmp_cells(n))
    }

    fn get(&self, _cx: &mut Cx, index: usize) -> Result<Option<Value>> {
        let mut current = Some(Arc::new(self.clone()));
        let mut i = index;
        while let Some(node) = current {
            let Some(car) = &node.car else {
                return Ok(None);
            };
            if i == 0 {
                return Ok(Some(car.clone()));
            }
            i -= 1;
            current = node.cdr.as_ref().cloned();
        }
        Ok(None)
    }

    fn for_each(
        &self,
        _cx: &mut Cx,
        limit: Option<usize>,
        visit: &mut dyn FnMut(&Value),
    ) -> Result<()> {
        if matches!(limit, Some(0)) {
            return Ok(());
        }

        let mut current = Some(Arc::new(self.clone()));
        let mut count = 0usize;
        while let Some(node) = current {
            let Some(car) = &node.car else {
                return Ok(());
            };
            if matches!(limit, Some(max) if count >= max) {
                return Ok(());
            }
            visit(car);
            count += 1;
            current = node.cdr.as_ref().cloned();
        }
        Ok(())
    }
}
