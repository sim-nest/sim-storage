use std::{cmp::Ordering, sync::Arc};

use sim_kernel::{
    Cx, DefaultFactory, EagerPolicy, Expr, Factory, ListBackend, ListValue, NumberLiteral,
    ObjectEncoding, Symbol, read_construct_capability,
};

use crate::{
    ConsBackend, ConsList, ConsListDescriptor, cons_list_class_symbol, install_cons_list_lib,
};

fn eval_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
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
