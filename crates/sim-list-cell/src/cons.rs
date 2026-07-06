//! The [`ConsList`] object: a singly linked, shared cons-cell list that
//! implements the kernel list value and object-encoding contracts.

use std::{cmp::Ordering, sync::Arc};

use sim_kernel::{
    CORE_LIST_CLASS_ID, ClassRef, Cx, Error, Expr, LengthResult, ListValue, Object, ObjectEncode,
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
    /// The remainder of the list, or `None` for the empty list.
    cdr: Option<Rest>,
}

/// The remainder of a [`ConsList`] cell: either another native cons node (a
/// cheap shared-pointer tail) or a foreign list value kept lazily.
#[derive(Clone)]
enum Rest {
    /// A native cons node; a `cdr` is a cheap [`Arc`] pointer clone.
    Cons(Arc<ConsList>),
    /// A foreign list value. Consing onto a non-`ConsList` tail keeps the tail
    /// as-is rather than materializing its (possibly unbounded) spine, so the
    /// laziness of an `iter`/`lazy` tail is preserved.
    Foreign(Value),
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
            cdr: Some(Rest::Cons(cdr)),
        }
    }

    /// Builds a non-empty cell prepending `car` onto a foreign list `tail`.
    ///
    /// The tail is kept as an opaque list value rather than materialized, so
    /// consing onto a lazy or unbounded list stays lazy.
    pub fn cell_foreign(car: Value, tail: Value) -> Self {
        Self {
            car: Some(car),
            cdr: Some(Rest::Foreign(tail)),
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
}

/// Reports the tail value referenced by a foreign cell as a list, or a type
/// mismatch if it is not one.
fn foreign_as_list(value: &Value) -> Result<&dyn ListValue> {
    value.object().as_list().ok_or(Error::TypeMismatch {
        expected: "list",
        found: "non-list",
    })
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
            Some(Rest::Cons(next)) => Ok(Some(cx.factory().opaque(next.clone())?)),
            Some(Rest::Foreign(tail)) => Ok(Some(tail.clone())),
            None => Ok(None),
        }
    }

    fn len(&self, cx: &mut Cx) -> Result<LengthResult> {
        if self.car.is_none() {
            return Ok(LengthResult::Known(0));
        }
        let mut count = 1usize;
        let mut rest = self.cdr.clone();
        loop {
            match rest {
                None => return Ok(LengthResult::Known(count)),
                Some(Rest::Cons(node)) => {
                    if node.car.is_none() {
                        return Ok(LengthResult::Known(count));
                    }
                    count += 1;
                    rest = node.cdr.clone();
                }
                Some(Rest::Foreign(tail)) => {
                    return Ok(match foreign_as_list(&tail)?.len(cx)? {
                        LengthResult::Known(k) => LengthResult::Known(count + k),
                        LengthResult::Unknown => LengthResult::Unknown,
                    });
                }
            }
        }
    }

    fn len_cmp(&self, cx: &mut Cx, n: usize) -> Result<Ordering> {
        if self.car.is_none() {
            return Ok(0usize.cmp(&n));
        }
        let mut count = 1usize;
        if count > n {
            return Ok(Ordering::Greater);
        }
        let mut rest = self.cdr.clone();
        loop {
            match rest {
                None => return Ok(count.cmp(&n)),
                Some(Rest::Cons(node)) => {
                    if node.car.is_none() {
                        return Ok(count.cmp(&n));
                    }
                    count += 1;
                    if count > n {
                        return Ok(Ordering::Greater);
                    }
                    rest = node.cdr.clone();
                }
                Some(Rest::Foreign(tail)) => {
                    // total len = count + tail_len; compare against n = count +
                    // (n - count), so it suffices to compare the tail against
                    // the residual budget.
                    return foreign_as_list(&tail)?.len_cmp(cx, n - count);
                }
            }
        }
    }

    fn get(&self, cx: &mut Cx, index: usize) -> Result<Option<Value>> {
        let mut node_car = self.car.clone();
        let mut rest = self.cdr.clone();
        let mut i = index;
        loop {
            let Some(car) = node_car else {
                return Ok(None);
            };
            if i == 0 {
                return Ok(Some(car));
            }
            i -= 1;
            match rest {
                None => return Ok(None),
                Some(Rest::Cons(node)) => {
                    node_car = node.car.clone();
                    rest = node.cdr.clone();
                }
                Some(Rest::Foreign(tail)) => {
                    return foreign_as_list(&tail)?.get(cx, i);
                }
            }
        }
    }

    fn for_each(
        &self,
        cx: &mut Cx,
        limit: Option<usize>,
        visit: &mut dyn FnMut(&Value),
    ) -> Result<()> {
        if matches!(limit, Some(0)) {
            return Ok(());
        }

        let mut node_car = self.car.clone();
        let mut rest = self.cdr.clone();
        let mut count = 0usize;
        loop {
            let Some(car) = node_car else {
                return Ok(());
            };
            if matches!(limit, Some(max) if count >= max) {
                return Ok(());
            }
            visit(&car);
            count += 1;
            match rest {
                None => return Ok(()),
                Some(Rest::Cons(node)) => {
                    node_car = node.car.clone();
                    rest = node.cdr.clone();
                }
                Some(Rest::Foreign(tail)) => {
                    let remaining = limit.map(|max| max - count);
                    return foreign_as_list(&tail)?.for_each(cx, remaining, visit);
                }
            }
        }
    }
}
