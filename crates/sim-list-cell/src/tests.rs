use std::{
    any::Any,
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
};

use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, Expr, Factory, LengthResult, ListBackend, ListValue,
    NumberLiteral, Object, ObjectCompat, ObjectEncoding, Result, Symbol, Value,
    read_construct_capability,
};

use crate::{
    ConsBackend, ConsList, ConsListDescriptor, cons_list_class_symbol, install_cons_list_lib,
};

fn eval_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

/// A test-only unbounded list that records how many heads have been forced, so
/// a test can assert that consing onto it does not realize its spine.
struct CountingInfiniteList {
    value: Value,
    pulls: Arc<AtomicUsize>,
}

impl Object for CountingInfiniteList {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("counting-infinite".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ObjectCompat for CountingInfiniteList {
    fn truth(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(true)
    }

    fn as_list(&self) -> Option<&dyn ListValue> {
        Some(self)
    }
}

impl ListValue for CountingInfiniteList {
    fn is_empty(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(false)
    }

    fn car(&self, _cx: &mut Cx) -> Result<Option<Value>> {
        self.pulls.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(Some(self.value.clone()))
    }

    fn cdr(&self, cx: &mut Cx) -> Result<Option<Value>> {
        cx.factory()
            .opaque(Arc::new(CountingInfiniteList {
                value: self.value.clone(),
                pulls: self.pulls.clone(),
            }))
            .map(Some)
    }

    fn len(&self, _cx: &mut Cx) -> Result<LengthResult> {
        Ok(LengthResult::Unknown)
    }

    fn len_cmp(&self, _cx: &mut Cx, _n: usize) -> Result<Ordering> {
        Ok(Ordering::Greater)
    }

    fn get(&self, cx: &mut Cx, index: usize) -> Result<Option<Value>> {
        // Force one head per step so a caller can observe how far it walked.
        let mut i = index;
        let mut head = self.car(cx)?;
        let mut tail = self.cdr(cx)?;
        while let Some(value) = head {
            if i == 0 {
                return Ok(Some(value));
            }
            i -= 1;
            match tail.as_ref().and_then(|node| node.object().as_list()) {
                Some(list) => {
                    head = list.car(cx)?;
                    tail = list.cdr(cx)?;
                }
                None => return Ok(None),
            }
        }
        Ok(None)
    }
}

fn number(text: &str) -> sim_kernel::Value {
    DefaultFactory
        .number_literal(Symbol::qualified("numbers", "f64"), text.to_owned())
        .unwrap()
}

#[test]
fn cons_list_len_cmp_and_walk() {
    let mut cx = eval_cx();
    let xs = ConsList::from_vec(vec![number("1"), number("2"), number("3")]);
    assert_eq!(xs.len_cmp(&mut cx, 2).unwrap(), Ordering::Greater);
    assert_eq!(xs.len_cmp(&mut cx, 3).unwrap(), Ordering::Equal);
    assert_eq!(
        xs.get(&mut cx, 1)
            .unwrap()
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "2".to_owned(),
        })
    );
}

#[test]
fn cons_backend_prepends_and_installs() {
    let mut cx = eval_cx();
    install_cons_list_lib(&mut cx).unwrap();
    cx.list_registry_mut().set_active("cons").unwrap();

    let backend = ConsBackend;
    let tail = backend
        .new_list(&mut cx, vec![number("2"), number("3")])
        .unwrap();
    let list = backend.new_cons(&mut cx, number("1"), tail).unwrap();
    let list = list.object().as_list().unwrap();

    assert_eq!(
        list.len(&mut cx).unwrap(),
        sim_kernel::LengthResult::Known(3)
    );
    assert_eq!(
        list.car(&mut cx)
            .unwrap()
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "1".to_owned(),
        })
    );
}

#[test]
fn cons_onto_unbounded_tail_stays_lazy() {
    // Regression guard for F41: consing onto a non-`ConsList` tail must wrap it
    // rather than materialize its spine, so an unbounded tail is not realized.
    let mut cx = eval_cx();
    let pulls = Arc::new(AtomicUsize::new(0));
    let one = number("1");
    let tail = cx
        .factory()
        .opaque(Arc::new(CountingInfiniteList {
            value: one,
            pulls: pulls.clone(),
        }))
        .unwrap();

    let value = ConsBackend.new_cons(&mut cx, number("0"), tail).unwrap();
    // Consing forced no heads of the unbounded tail.
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 0);

    let list = value.object().as_list().unwrap();
    assert_eq!(
        list.get(&mut cx, 0)
            .unwrap()
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "0".to_owned(),
        })
    );
    // Reading only the consed head still forces nothing from the tail.
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 0);

    // The tail is a live view: length is unknown and len_cmp terminates.
    assert_eq!(list.len(&mut cx).unwrap(), LengthResult::Unknown);
    assert_eq!(list.len_cmp(&mut cx, 100).unwrap(), Ordering::Greater);

    // Walking into the tail forces exactly the heads requested.
    let second = list.get(&mut cx, 1).unwrap().unwrap();
    assert_eq!(
        second.object().as_expr(&mut cx).unwrap(),
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("numbers", "f64"),
            canonical: "1".to_owned(),
        })
    );
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 1);
}

#[test]
fn cons_list_citizen_round_trips_as_descriptor() {
    let mut cx = eval_cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let list = ConsList::from_vec(vec![number("1"), number("2")]);
    let original = cx.factory().opaque(list).unwrap();

    sim_citizen::check_value_fixture(&mut cx, original.clone()).unwrap();

    let ObjectEncoding::Constructor { args, .. } = original
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .unwrap()
    else {
        panic!("expected constructor encoding");
    };
    let args = args
        .iter()
        .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();
    let decoded = cx.read_construct(&cons_list_class_symbol(), args).unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<ConsListDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_list().is_none());
}
