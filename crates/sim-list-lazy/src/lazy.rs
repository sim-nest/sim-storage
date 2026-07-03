//! The [`LazyConsList`] object: a cons list whose head and tail are computed by
//! deferred closures ([`HeadFn`]/[`TailFn`]) and cached on first force, plus the
//! [`unfold`] generator for building such lists from a seed state.

use std::{
    cmp::Ordering,
    sync::{Arc, OnceLock},
};

use sim_kernel::{
    CORE_LIST_CLASS_ID, ClassRef, Cx, Error, Expr, LengthResult, ListValue, Object, ObjectEncode,
    ObjectEncoding, Result, Symbol, Value, force_list_to_vec, spine_len_cmp,
};

use crate::citizen::lazy_cons_list_class_symbol;

/// A shared closure computing a [`LazyConsList`] head on demand.
pub type HeadFn = Arc<dyn Fn(&mut Cx) -> Result<Value> + Send + Sync>;
/// A shared closure computing a [`LazyConsList`] tail on demand, yielding the
/// next list node or `None` at the end of the list.
pub type TailFn = Arc<dyn Fn(&mut Cx) -> Result<Option<Value>> + Send + Sync>;
/// The step function of [`unfold`]: given the current seed `S`, produces the
/// next element and the seed for the remainder of the list.
pub type UnfoldStep<S> = dyn Fn(&mut Cx, &S) -> Result<(Value, S)> + Send + Sync;

/// A cons list whose head and tail are computed by deferred closures.
///
/// A node is either empty or a cell carrying a [`HeadFn`] and [`TailFn`]. The
/// head and tail are forced at most once and cached (via [`OnceLock`]), so
/// repeated reads of the same node reuse the first result. This makes
/// `LazyConsList` suitable for unbounded or expensive-to-produce sequences,
/// including the generators built by [`unfold`]. Length is reported as
/// [`LengthResult::Unknown`] since the spine is only known by walking it.
///
/// `LazyConsList` is the concrete object behind the `lazy`
/// [`ListBackend`](sim_kernel::ListBackend) ([`crate::LazyBackend`]).
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, ListValue, LengthResult};
/// use sim_list_lazy::LazyConsList;
///
/// let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
///
/// // The empty list.
/// let empty = LazyConsList::empty();
/// assert!(empty.is_empty(&mut cx).unwrap());
/// assert_eq!(empty.len(&mut cx).unwrap(), LengthResult::Unknown);
///
/// // A single cell whose head is forced on demand.
/// let head = cx.factory().bool(true).unwrap();
/// let one = LazyConsList::new(
///     move |_cx| Ok(head.clone()),
///     |_cx| Ok(None),
/// );
/// assert!(!one.is_empty(&mut cx).unwrap());
/// assert!(one.car(&mut cx).unwrap().is_some());
/// assert!(one.cdr(&mut cx).unwrap().is_none());
/// ```
pub struct LazyConsList {
    /// Whether this node is the empty list (no head or tail).
    empty: bool,
    /// The closure producing the head, or `None` for the empty list.
    head_fn: Option<HeadFn>,
    /// The closure producing the tail, or `None` for the empty list.
    tail_fn: Option<TailFn>,
    /// The forced head, computed once on first access.
    head_cache: OnceLock<Result<Value>>,
    /// The forced tail, computed once on first access.
    tail_cache: OnceLock<Result<Option<Value>>>,
}

impl Clone for LazyConsList {
    fn clone(&self) -> Self {
        let head_cache = OnceLock::new();
        if let Some(value) = self.head_cache.get() {
            let _ = head_cache.set(value.clone());
        }
        let tail_cache = OnceLock::new();
        if let Some(value) = self.tail_cache.get() {
            let _ = tail_cache.set(value.clone());
        }
        Self {
            empty: self.empty,
            head_fn: self.head_fn.clone(),
            tail_fn: self.tail_fn.clone(),
            head_cache,
            tail_cache,
        }
    }
}

impl LazyConsList {
    /// Returns the empty lazy list (a node with no head and no tail).
    pub fn empty() -> Self {
        Self {
            empty: true,
            head_fn: None,
            tail_fn: None,
            head_cache: OnceLock::new(),
            tail_cache: OnceLock::new(),
        }
    }

    /// Builds a non-empty cell from closures computing the head and tail on
    /// demand. Convenience over [`LazyConsList::cell`] that wraps each closure
    /// in an [`Arc`].
    pub fn new(
        head_fn: impl Fn(&mut Cx) -> Result<Value> + Send + Sync + 'static,
        tail_fn: impl Fn(&mut Cx) -> Result<Option<Value>> + Send + Sync + 'static,
    ) -> Self {
        Self::cell(Arc::new(head_fn), Arc::new(tail_fn))
    }

    /// Builds a non-empty cell from already-shared [`HeadFn`] and [`TailFn`]
    /// closures.
    pub fn cell(head_fn: HeadFn, tail_fn: TailFn) -> Self {
        Self {
            empty: false,
            head_fn: Some(head_fn),
            tail_fn: Some(tail_fn),
            head_cache: OnceLock::new(),
            tail_cache: OnceLock::new(),
        }
    }

    fn cached_head(&self, cx: &mut Cx) -> Result<Value> {
        let Some(head_fn) = self.head_fn.as_ref() else {
            return Err(Error::Eval("lazy list head missing".to_owned()));
        };
        self.head_cache.get_or_init(|| head_fn(cx)).clone()
    }

    fn cached_tail(&self, cx: &mut Cx) -> Result<Option<Value>> {
        let Some(tail_fn) = self.tail_fn.as_ref() else {
            return Err(Error::Eval("lazy list tail missing".to_owned()));
        };
        self.tail_cache.get_or_init(|| tail_fn(cx)).clone()
    }
}

impl Object for LazyConsList {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(if self.empty {
            "lazy-cons[]".to_owned()
        } else {
            "lazy-cons[...]".to_owned()
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for LazyConsList {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = lazy_cons_list_class_symbol();
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
        Ok(!self.empty)
    }
    fn as_list(&self) -> Option<&dyn ListValue> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for LazyConsList {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        let items = force_list_to_vec(cx, self, "list/LazyConsList citizen")?
            .into_iter()
            .map(|value| value.object().as_expr(cx))
            .collect::<Result<Vec<_>>>()?;
        Ok(ObjectEncoding::Constructor {
            class: lazy_cons_list_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                crate::citizen::expr_items::encode(&items),
            ],
        })
    }
}

impl sim_citizen::Citizen for LazyConsList {
    fn citizen_symbol() -> Symbol {
        lazy_cons_list_class_symbol()
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

impl ListValue for LazyConsList {
    fn is_empty(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(self.empty)
    }

    fn car(&self, cx: &mut Cx) -> Result<Option<Value>> {
        if self.empty {
            return Ok(None);
        }
        self.cached_head(cx).map(Some)
    }

    fn cdr(&self, cx: &mut Cx) -> Result<Option<Value>> {
        if self.empty {
            return Ok(None);
        }
        self.cached_tail(cx)
    }

    fn len(&self, _cx: &mut Cx) -> Result<LengthResult> {
        Ok(LengthResult::Unknown)
    }

    fn len_cmp(&self, cx: &mut Cx, n: usize) -> Result<Ordering> {
        spine_len_cmp(cx, self, n)
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

/// Builds a [`LazyConsList`] by repeatedly applying `step` to a seed.
///
/// Starting from `seed`, each `step` call produces the next element and the
/// seed for the rest of the list; the list is generated lazily, one cell at a
/// time, as it is traversed. Since `step` never signals termination, `unfold`
/// produces an unbounded list; bound traversal with a limit (for example via
/// [`ListValue::for_each`]) when consuming it.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, ListValue, Value};
/// use sim_list_lazy::unfold;
///
/// let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
///
/// // An unbounded stream of `true` values; take the first three.
/// let xs = unfold((), |cx, _seed| Ok((cx.factory().bool(true)?, ())));
/// let mut seen = 0usize;
/// xs.for_each(&mut cx, Some(3), &mut |_value: &Value| seen += 1)
///     .unwrap();
/// assert_eq!(seen, 3);
/// ```
pub fn unfold<S>(
    seed: S,
    step: impl Fn(&mut Cx, &S) -> Result<(Value, S)> + Send + Sync + 'static,
) -> Arc<LazyConsList>
where
    S: Clone + Send + Sync + 'static,
{
    lazy_unfold(seed, Arc::new(step))
}

fn lazy_unfold<S>(seed: S, step: Arc<UnfoldStep<S>>) -> Arc<LazyConsList>
where
    S: Clone + Send + Sync + 'static,
{
    let head_seed = seed.clone();
    let tail_seed = seed;
    let head_step = step.clone();
    let tail_step = step.clone();
    Arc::new(LazyConsList::new(
        move |cx| Ok((head_step)(cx, &head_seed)?.0),
        move |cx| {
            let (_, next_seed) = (tail_step)(cx, &tail_seed)?;
            Ok(Some(
                cx.factory()
                    .opaque(lazy_unfold(next_seed, tail_step.clone()))?,
            ))
        },
    ))
}
