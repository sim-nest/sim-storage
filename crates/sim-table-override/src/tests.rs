use std::sync::Arc;

use sim_kernel::{
    Args, CapabilitySet, Cx, DefaultFactory, Expr, NoopEvalPolicy, ObjectEncoding, ReadPolicy,
    Symbol, TrustLevel, Value, read_construct_capability,
};

use crate::{OverrideTable, construct_override_table, install_override_table_lib};

fn test_cx() -> Cx {
    Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory))
}

fn table(cx: &mut Cx, entries: &[(&str, Value)]) -> Value {
    cx.new_table(
        entries
            .iter()
            .map(|(key, value)| (Symbol::new(*key), value.clone()))
            .collect(),
    )
    .unwrap()
}

fn value_expr(cx: &mut Cx, value: Value) -> Expr {
    value.object().as_expr(cx).unwrap()
}

fn get_expr(cx: &mut Cx, table: &dyn sim_kernel::Table, key: &str) -> Expr {
    let value = table.get(cx, Symbol::new(key)).unwrap();
    value_expr(cx, value)
}

fn read_policy(capabilities: &[sim_kernel::CapabilityName]) -> ReadPolicy {
    ReadPolicy {
        trust: TrustLevel::Untrusted,
        capabilities: capabilities
            .iter()
            .cloned()
            .fold(CapabilitySet::new(), |set, capability| {
                set.grant(capability)
            }),
    }
}

#[test]
fn override_table_front_shadows_and_writes_to_front() {
    let mut cx = test_cx();
    let back_a = cx.factory().string("back-a".to_owned()).unwrap();
    let back_b = cx.factory().string("back-b".to_owned()).unwrap();
    let back = table(&mut cx, &[("a", back_a), ("b", back_b)]);
    let front_b = cx.factory().string("front-b".to_owned()).unwrap();
    let front_c = cx.factory().string("front-c".to_owned()).unwrap();
    let front = table(&mut cx, &[("b", front_b)]);

    let value = construct_override_table(&mut cx, vec![front.clone(), back.clone()]).unwrap();
    let override_table = value.object().as_table_impl().unwrap();

    assert_eq!(
        override_table
            .get(&mut cx, Symbol::new("b"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        sim_kernel::Expr::String("front-b".to_owned())
    );
    assert_eq!(
        override_table
            .get(&mut cx, Symbol::new("a"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        sim_kernel::Expr::String("back-a".to_owned())
    );

    override_table
        .set(&mut cx, Symbol::new("c"), front_c)
        .unwrap();
    assert!(
        front
            .object()
            .as_table_impl()
            .unwrap()
            .has(&mut cx, Symbol::new("c"))
            .unwrap()
    );
    assert!(
        !back
            .object()
            .as_table_impl()
            .unwrap()
            .has(&mut cx, Symbol::new("c"))
            .unwrap()
    );
}

#[test]
fn override_table_del_masks_front_and_lower_layers_until_set() {
    let mut cx = test_cx();
    let front_value = cx.factory().string("front-b".to_owned()).unwrap();
    let back_value = cx.factory().string("back-b".to_owned()).unwrap();
    let replacement = cx.factory().string("new-b".to_owned()).unwrap();
    let front = table(&mut cx, &[("b", front_value)]);
    let back = table(&mut cx, &[("b", back_value)]);
    let value = construct_override_table(&mut cx, vec![front, back]).unwrap();
    let override_table = value.object().as_table_impl().unwrap();

    let removed = override_table.del(&mut cx, Symbol::new("b")).unwrap();
    assert_eq!(
        value_expr(&mut cx, removed),
        Expr::String("front-b".to_owned())
    );
    assert_eq!(get_expr(&mut cx, override_table, "b"), Expr::Nil);
    assert!(!override_table.has(&mut cx, Symbol::new("b")).unwrap());
    assert!(
        !override_table
            .keys(&mut cx)
            .unwrap()
            .contains(&Symbol::new("b"))
    );
    assert!(
        !override_table
            .entries(&mut cx)
            .unwrap()
            .iter()
            .any(|(key, _)| key == &Symbol::new("b"))
    );

    override_table
        .set(&mut cx, Symbol::new("b"), replacement)
        .unwrap();
    assert_eq!(
        get_expr(&mut cx, override_table, "b"),
        Expr::String("new-b".to_owned())
    );
    assert!(override_table.has(&mut cx, Symbol::new("b")).unwrap());
    assert!(
        override_table
            .keys(&mut cx)
            .unwrap()
            .contains(&Symbol::new("b"))
    );
}

#[test]
fn override_table_del_masks_base_only_key() {
    let mut cx = test_cx();
    let back_value = cx.factory().string("back-a".to_owned()).unwrap();
    let front = table(&mut cx, &[]);
    let back = table(&mut cx, &[("a", back_value)]);
    let value = construct_override_table(&mut cx, vec![front, back]).unwrap();
    let override_table = value.object().as_table_impl().unwrap();

    let removed = override_table.del(&mut cx, Symbol::new("a")).unwrap();
    assert_eq!(value_expr(&mut cx, removed), Expr::Nil);
    assert_eq!(get_expr(&mut cx, override_table, "a"), Expr::Nil);
    assert!(!override_table.has(&mut cx, Symbol::new("a")).unwrap());
    assert!(
        !override_table
            .keys(&mut cx)
            .unwrap()
            .contains(&Symbol::new("a"))
    );
}

#[test]
fn override_table_clear_masks_visible_keys() {
    let mut cx = test_cx();
    let front_value = cx.factory().string("front-b".to_owned()).unwrap();
    let back_value = cx.factory().string("back-a".to_owned()).unwrap();
    let replacement = cx.factory().string("new-a".to_owned()).unwrap();
    let front = table(&mut cx, &[("b", front_value)]);
    let back = table(&mut cx, &[("a", back_value)]);
    let value = construct_override_table(&mut cx, vec![front, back]).unwrap();
    let override_table = value.object().as_table_impl().unwrap();

    override_table.clear(&mut cx).unwrap();

    assert_eq!(override_table.keys(&mut cx).unwrap(), Vec::<Symbol>::new());
    assert!(override_table.entries(&mut cx).unwrap().is_empty());
    assert_eq!(override_table.len(&mut cx).unwrap(), 0);
    assert_eq!(get_expr(&mut cx, override_table, "a"), Expr::Nil);
    assert_eq!(get_expr(&mut cx, override_table, "b"), Expr::Nil);

    override_table
        .set(&mut cx, Symbol::new("a"), replacement)
        .unwrap();
    assert_eq!(
        get_expr(&mut cx, override_table, "a"),
        Expr::String("new-a".to_owned())
    );
    assert!(override_table.has(&mut cx, Symbol::new("a")).unwrap());
}

#[test]
fn override_table_constructor_class_and_read_construct_share_path() {
    let mut cx = test_cx();
    install_override_table_lib(&mut cx).unwrap();

    let front_value = cx.factory().bool(true).unwrap();
    let back_value = cx.factory().nil().unwrap();
    let front = table(&mut cx, &[("front", front_value)]);
    let back = table(&mut cx, &[("back", back_value)]);

    let via_class = cx
        .call_class(
            &Symbol::new("OverrideTable"),
            Args::new(vec![front.clone(), back.clone()]),
        )
        .unwrap();
    assert!(via_class.object().as_table_impl().is_some());

    let denied = cx.read_construct(
        &Symbol::new("OverrideTable"),
        vec![front.clone(), back.clone()],
    );
    assert!(matches!(
        denied,
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == read_construct_capability()
    ));

    cx.grant(read_construct_capability());
    let via_read = cx
        .read_construct(&Symbol::new("OverrideTable"), vec![front, back])
        .unwrap();
    let encoding = via_read
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .unwrap();
    assert!(matches!(
        encoding,
        ObjectEncoding::Constructor { class, args }
            if class == Symbol::new("OverrideTable") && args.len() == 2
    ));

    let _ = read_policy(&[read_construct_capability()]);
}

#[test]
fn override_table_rejects_non_table_layers() {
    let cx = test_cx();
    assert!(matches!(
        OverrideTable::new(vec![cx.factory().bool(true).unwrap()]),
        Err(sim_kernel::Error::Eval(message)) if message.contains("every layer must be a table")
    ));
}
