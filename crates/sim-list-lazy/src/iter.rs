//! The [`LazyIterList`] object: a list whose spine is driven by an underlying
//! iterator, forcing and caching elements on demand as the list is traversed.

use std::{
    cmp::Ordering,
    sync::{Arc, Mutex},
};

use sim_kernel::{
    CORE_LIST_CLASS_ID, ClassRef, Cx, Error, Expr, LengthResult, ListValue, Object, ObjectEncode,
    ObjectEncoding, Result, Symbol, Value, force_list_to_vec,
};

use crate::citizen::lazy_iter_list_class_symbol;

/// The iterator type driving a [`LazyIterList`] spine: yields each element as a
/// fallible [`Value`], and is [`Send`] so the list can move across threads.
pub type ValueIter = dyn Iterator<Item = Result<Value>> + Send;

enum IterDriver {
    Iter(Box<ValueIter>),
    List(ListCursor),
}

impl IterDriver {
    fn next_value(&mut self, cx: &mut Cx) -> Result<Option<Value>> {
        match self {
            Self::Iter(iter) => iter.next().transpose(),
            Self::List(cursor) => cursor.next_value(cx),
        }
    }
}

struct ListCursor {
    next: Option<Value>,
}

impl ListCursor {
    fn new(next: Value) -> Self {
        Self { next: Some(next) }
    }

    fn next_value(&mut self, cx: &mut Cx) -> Result<Option<Value>> {
        let Some(node) = self.next.take() else {
            return Ok(None);
        };
        let Some(list) = node.object().as_list() else {
            return Err(Error::Eval("list cdr did not yield a list".to_owned()));
        };
        if list.is_empty(cx)? {
            return Ok(None);
        }
        let head = list
            .car(cx)?
            .ok_or_else(|| Error::Eval("list car missing for non-empty list".to_owned()))?;
        self.next = list.cdr(cx)?;
        Ok(Some(head))
    }
}

struct IterState {
    driver: IterDriver,
    buffer: Vec<Value>,
    done: bool,
}

impl IterState {
    fn new(iter: Box<ValueIter>) -> Self {
        Self {
            driver: IterDriver::Iter(iter),
            buffer: Vec::new(),
            done: false,
        }
    }

    fn with_prefix(first: Value, tail: Value) -> Self {
        Self {
            driver: IterDriver::List(ListCursor::new(tail)),
            buffer: vec![first],
            done: false,
        }
    }

    fn fill_to(&mut self, cx: &mut Cx, need: usize) -> Result<usize> {
        while self.buffer.len() < need && !self.done {
            match self.driver.next_value(cx)? {
                Some(item) => self.buffer.push(item),
                None => self.done = true,
            }
        }
        Ok(self.buffer.len().min(need))
    }
}

/// A list whose spine is driven by an underlying iterator.
///
/// Elements are forced and cached as the list is traversed, so each element is
/// produced at most once even across shared clones. The shared iterator state
/// is held behind an [`Arc`]/[`Mutex`]; a [`cdr`](ListValue::cdr) yields a
/// view of the same buffer with its `start` offset advanced by one, so tails
/// are cheap and share already-forced elements. Length is reported as
/// [`LengthResult::Unknown`] until the spine is walked.
///
/// `LazyIterList` is the concrete object behind the `iter`
/// [`ListBackend`](sim_kernel::ListBackend) ([`crate::IterBackend`]).
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, ListValue, LengthResult};
/// use sim_list_lazy::LazyIterList;
///
/// let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
///
/// // An empty iterator yields the empty list.
/// let empty = LazyIterList::new(Box::new(std::iter::empty()));
/// assert!(empty.is_empty(&mut cx).unwrap());
/// assert_eq!(empty.len(&mut cx).unwrap(), LengthResult::Unknown);
///
/// // A two-element iterator, traversed by head and tail.
/// let a = cx.factory().bool(true).unwrap();
/// let b = cx.factory().bool(false).unwrap();
/// let xs = LazyIterList::new(Box::new(vec![Ok(a), Ok(b)].into_iter()));
/// assert!(xs.car(&mut cx).unwrap().is_some());
/// let tail = xs.cdr(&mut cx).unwrap().unwrap();
/// let tail = tail.object().as_list().unwrap();
/// assert!(tail.car(&mut cx).unwrap().is_some());
/// assert!(!tail.is_empty(&mut cx).unwrap());
/// ```
#[derive(Clone)]
pub struct LazyIterList {
    state: Arc<Mutex<IterState>>,
    start: usize,
}

impl LazyIterList {
    /// Builds a lazy list whose elements are produced by `iter` on demand.
    pub fn new(iter: Box<ValueIter>) -> Self {
        Self {
            state: Arc::new(Mutex::new(IterState::new(iter))),
            start: 0,
        }
    }

    /// Builds a lazy list with `first` as its head, streaming the rest from the
    /// existing list `tail`.
    pub fn prepend(first: Value, tail: Value) -> Self {
        Self {
            state: Arc::new(Mutex::new(IterState::with_prefix(first, tail))),
            start: 0,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, IterState>> {
        self.state
            .lock()
            .map_err(|_| Error::Eval("list/lazy lock poisoned".to_owned()))
    }
}

impl Object for LazyIterList {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("lazy-iter[...]".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for LazyIterList {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = lazy_iter_list_class_symbol();
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
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_list(&self) -> Option<&dyn ListValue> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for LazyIterList {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        let items = force_list_to_vec(cx, self, "list/LazyIterList citizen")?
            .into_iter()
            .map(|value| value.object().as_expr(cx))
            .collect::<Result<Vec<_>>>()?;
        Ok(ObjectEncoding::Constructor {
            class: lazy_iter_list_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                crate::citizen::expr_items::encode(&items),
            ],
        })
    }
}

impl sim_citizen::Citizen for LazyIterList {
    fn citizen_symbol() -> Symbol {
        lazy_iter_list_class_symbol()
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

impl ListValue for LazyIterList {
    fn is_empty(&self, cx: &mut Cx) -> Result<bool> {
        let mut state = self.lock()?;
        let have = state.fill_to(cx, self.start + 1)?;
        Ok(have <= self.start)
    }

    fn car(&self, cx: &mut Cx) -> Result<Option<Value>> {
        let mut state = self.lock()?;
        let have = state.fill_to(cx, self.start + 1)?;
        if have <= self.start {
            Ok(None)
        } else {
            Ok(Some(state.buffer[self.start].clone()))
        }
    }

    fn cdr(&self, cx: &mut Cx) -> Result<Option<Value>> {
        if self.is_empty(cx)? {
            return Ok(None);
        }
        cx.factory()
            .opaque(Arc::new(Self {
                state: self.state.clone(),
                start: self.start + 1,
            }))
            .map(Some)
    }

    fn len(&self, _cx: &mut Cx) -> Result<LengthResult> {
        Ok(LengthResult::Unknown)
    }

    fn len_cmp(&self, cx: &mut Cx, n: usize) -> Result<Ordering> {
        let mut state = self.lock()?;
        let need = self.start.saturating_add(n).saturating_add(1);
        let have = state.fill_to(cx, need)?;
        let remaining = have.saturating_sub(self.start);
        if remaining > n {
            Ok(Ordering::Greater)
        } else {
            Ok(remaining.cmp(&n))
        }
    }

    fn get(&self, cx: &mut Cx, index: usize) -> Result<Option<Value>> {
        let mut head = self.car(cx)?;
        let mut tail = self.cdr(cx)?;
        let mut i = index;
        while let Some(value) = head {
            if i == 0 {
                return Ok(Some(value));
            }
            i -= 1;
            match tail.as_ref().and_then(|node| node.object().as_list()) {
                Some(list) if !list.is_empty(cx)? => {
                    head = list.car(cx)?;
                    tail = list.cdr(cx)?;
                }
                _ => return Ok(None),
            }
        }
        Ok(None)
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

        let mut count = 0usize;
        let mut head = self.car(cx)?;
        let mut tail = self.cdr(cx)?;
        while let Some(value) = head {
            if matches!(limit, Some(max) if count >= max) {
                return Ok(());
            }
            visit(&value);
            count += 1;
            match tail.as_ref().and_then(|node| node.object().as_list()) {
                Some(list) if !list.is_empty(cx)? => {
                    head = list.car(cx)?;
                    tail = list.cdr(cx)?;
                }
                _ => return Ok(()),
            }
        }
        Ok(())
    }
}
