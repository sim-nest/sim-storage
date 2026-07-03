use std::{
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
};

use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, LengthResult, ListBackend, ListSequence, ObjectEncoding,
    Value, read_construct_capability, seq_next,
};

use crate::{
    IterBackend, LazyBackend, LazyConsList, LazyConsListDescriptor, LazyIterList,
    LazyIterListDescriptor, install_lazy_list_lib, lazy_cons_list_class_symbol,
    lazy_iter_list_class_symbol, unfold,
};

fn test_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

fn num(cx: &mut Cx, n: i64) -> Value {
    cx.factory()
        .number_literal(
            sim_kernel::Symbol::qualified("numbers", "f64"),
            n.to_string(),
        )
        .unwrap()
}

fn rendered(cx: &mut Cx, value: &Value) -> String {
    value.object().display(cx).unwrap()
}

fn make_naturals(cx: &mut Cx) -> Value {
    cx.factory()
        .opaque(unfold(0usize, |cx, seed| {
            let value = cx.factory().number_literal(
                sim_kernel::Symbol::qualified("numbers", "f64"),
                seed.to_string(),
            )?;
            Ok((value, seed + 1))
        }))
        .unwrap()
}

#[test]
fn lazy_naturals_len_cmp_terminates() {
    let mut cx = test_cx();
    let nats = make_naturals(&mut cx);
    let list = nats.object().as_list().unwrap();

    assert_eq!(list.len(&mut cx).unwrap(), LengthResult::Unknown);
    assert_eq!(list.len_cmp(&mut cx, 5).unwrap(), Ordering::Greater);

    let prefix = list.to_vec(&mut cx, Some(3)).unwrap();
    assert_eq!(prefix.len(), 3);
    assert_eq!(
        prefix
            .into_iter()
            .map(|value| value.object().display(&mut cx).unwrap())
            .collect::<Vec<_>>(),
        vec!["0".to_owned(), "1".to_owned(), "2".to_owned()]
    );
}

#[test]
fn lazy_list_can_be_consumed_as_sequence() {
    let mut cx = test_cx();
    let nats = make_naturals(&mut cx);
    let sequence = ListSequence::new(nats);

    let first = seq_next(&mut cx, &sequence).unwrap().unwrap();
    let second = seq_next(&mut cx, &sequence).unwrap().unwrap();

    assert_eq!(rendered(&mut cx, first.value()), "0");
    assert_eq!(rendered(&mut cx, second.value()), "1");
}

#[test]
fn len_cmp_terminates_on_endless_lazy_cons() {
    let mut cx = test_cx();
    let looped = Arc::<LazyConsList>::new_cyclic(|weak| {
        let weak = weak.clone();
        LazyConsList::new(
            move |cx| cx.factory().bool(true),
            move |cx| Ok(Some(cx.factory().opaque(weak.upgrade().unwrap())?)),
        )
    });
    let value = cx.factory().opaque(looped).unwrap();
    let list = value.object().as_list().unwrap();

    assert_eq!(list.len_cmp(&mut cx, 100).unwrap(), Ordering::Greater);
}

#[test]
fn len_cmp_does_not_force_heads_on_lazy_cons() {
    let mut cx = test_cx();
    let head_forces = Arc::new(AtomicUsize::new(0));
    let looped = Arc::<LazyConsList>::new_cyclic(|weak| {
        let weak = weak.clone();
        let head_forces = head_forces.clone();
        LazyConsList::new(
            move |cx| {
                head_forces.fetch_add(1, AtomicOrdering::SeqCst);
                cx.factory().bool(true)
            },
            move |cx| Ok(Some(cx.factory().opaque(weak.upgrade().unwrap())?)),
        )
    });
    let value = cx.factory().opaque(looped).unwrap();
    let list = value.object().as_list().unwrap();

    assert_eq!(list.len_cmp(&mut cx, 5).unwrap(), Ordering::Greater);
    assert_eq!(head_forces.load(AtomicOrdering::SeqCst), 0);
}

#[test]
fn lazy_head_thunk_runs_once() {
    let mut cx = test_cx();
    let calls = Arc::new(AtomicUsize::new(0));
    let head_calls = calls.clone();
    let value = cx
        .factory()
        .opaque(Arc::new(LazyConsList::new(
            move |cx| {
                head_calls.fetch_add(1, AtomicOrdering::SeqCst);
                cx.factory().bool(true)
            },
            |_cx| Ok(None),
        )))
        .unwrap();
    let list = value.object().as_list().unwrap();

    assert!(list.car(&mut cx).unwrap().is_some());
    assert!(list.car(&mut cx).unwrap().is_some());
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
}

#[test]
fn lazy_backend_installs_and_builds_lists() {
    let mut cx = test_cx();
    install_lazy_list_lib(&mut cx).unwrap();
    cx.list_registry_mut().set_active("lazy").unwrap();

    let one = cx
        .factory()
        .number_literal(
            sim_kernel::Symbol::qualified("numbers", "f64"),
            "1".to_owned(),
        )
        .unwrap();
    let two = cx
        .factory()
        .number_literal(
            sim_kernel::Symbol::qualified("numbers", "f64"),
            "2".to_owned(),
        )
        .unwrap();
    let value = LazyBackend
        .new_list(&mut cx, vec![one.clone(), two.clone()])
        .unwrap();
    let list = value.object().as_list().unwrap();
    assert_eq!(list.len(&mut cx).unwrap(), LengthResult::Unknown);
    assert_eq!(list.len_cmp(&mut cx, 2).unwrap(), Ordering::Equal);
    assert_eq!(list.get(&mut cx, 0).unwrap(), Some(one));
    assert_eq!(list.get(&mut cx, 1).unwrap(), Some(two));
}

#[test]
fn lazy_iter_endless_len_cmp() {
    let mut cx = test_cx();
    let one = cx.factory().bool(true).unwrap();
    let ones = LazyIterList::new(Box::new(std::iter::repeat_with(move || Ok(one.clone()))));
    let list = cx.factory().opaque(Arc::new(ones)).unwrap();
    let list = list.object().as_list().unwrap();

    assert_eq!(list.len_cmp(&mut cx, 10).unwrap(), Ordering::Greater);
    let prefix = list.to_vec(&mut cx, Some(4)).unwrap();
    assert_eq!(prefix.len(), 4);
}

#[test]
fn len_cmp_pulls_at_most_n_plus_one_iter_heads() {
    let mut cx = test_cx();
    let pulls = Arc::new(AtomicUsize::new(0));
    let iter_pulls = pulls.clone();
    let one = cx.factory().bool(true).unwrap();
    let list = cx
        .factory()
        .opaque(Arc::new(LazyIterList::new(Box::new(
            std::iter::repeat_with(move || {
                iter_pulls.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(one.clone())
            }),
        ))))
        .unwrap();
    let list = list.object().as_list().unwrap();

    assert_eq!(list.len_cmp(&mut cx, 5).unwrap(), Ordering::Greater);
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 6);
}

#[test]
fn lazy_iter_heads_not_pulled_twice() {
    let mut cx = test_cx();
    let pulls = Arc::new(AtomicUsize::new(0));
    let iter_pulls = pulls.clone();
    let zero = num(&mut cx, 0);
    let one = num(&mut cx, 1);
    let two = num(&mut cx, 2);
    let list = cx
        .factory()
        .opaque(Arc::new(LazyIterList::new(Box::new(
            [zero.clone(), one.clone(), two]
                .into_iter()
                .map(move |value| {
                    iter_pulls.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(value)
                }),
        ))))
        .unwrap();
    let list = list.object().as_list().unwrap();

    let tail = list.cdr(&mut cx).unwrap().unwrap();
    let tail = tail.object().as_list().unwrap();

    let first = list.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &first), "0");
    let second = tail.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &second), "1");
    let first_again = list.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &first_again), "0");
    let second_again = tail.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &second_again), "1");
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 2);
}

#[test]
fn iter_backend_cons_preserves_lazy_tail() {
    let mut cx = test_cx();
    let pulls = Arc::new(AtomicUsize::new(0));
    let iter_pulls = pulls.clone();
    let one = num(&mut cx, 1);
    let tail = cx
        .factory()
        .opaque(Arc::new(LazyIterList::new(Box::new(
            std::iter::repeat_with(move || {
                iter_pulls.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(one.clone())
            }),
        ))))
        .unwrap();

    let zero = num(&mut cx, 0);
    let value = IterBackend.new_cons(&mut cx, zero, tail).unwrap();
    let list = value.object().as_list().unwrap();

    let head = list.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &head), "0");
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 0);
    let tail = list.cdr(&mut cx).unwrap().unwrap();
    let tail = tail.object().as_list().unwrap();
    let tail_head = tail.car(&mut cx).unwrap().unwrap();
    assert_eq!(rendered(&mut cx, &tail_head), "1");
    assert_eq!(pulls.load(AtomicOrdering::SeqCst), 1);
}

#[test]
fn lazy_cons_citizen_round_trips_as_descriptor() {
    let mut cx = test_cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let one = num(&mut cx, 1);
    let list = Arc::new(LazyConsList::new(move |_| Ok(one.clone()), |_cx| Ok(None)));
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
    let decoded = cx
        .read_construct(&lazy_cons_list_class_symbol(), args)
        .unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<LazyConsListDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_list().is_none());
}

#[test]
fn lazy_iter_citizen_round_trips_as_descriptor() {
    let mut cx = test_cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let items = vec![num(&mut cx, 1), num(&mut cx, 2)];
    let list = Arc::new(LazyIterList::new(Box::new(items.into_iter().map(Ok))));
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
    let decoded = cx
        .read_construct(&lazy_iter_list_class_symbol(), args)
        .unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<LazyIterListDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_list().is_none());
}

#[test]
fn lazy_cons_citizen_encoding_fails_closed_on_tail_error() {
    let mut cx = test_cx();
    let list = Arc::new(LazyConsList::new(
        |cx| cx.factory().bool(true),
        |_cx| Err(sim_kernel::Error::Eval("tail denied".to_owned())),
    ));
    let value = cx.factory().opaque(list).unwrap();

    let err = value
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .expect_err("tail error must stop descriptor encoding");
    assert!(err.to_string().contains("tail denied"));
}
