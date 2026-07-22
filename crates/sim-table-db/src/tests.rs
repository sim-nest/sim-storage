use std::sync::Arc;

use sim_kernel::{
    DefaultFactory, EagerPolicy, Expr, Object, ObjectCompat, ObjectEncoding, Symbol,
    read_construct_capability,
};

use crate::{
    DbDir, DbDirDescriptor, db_dir_class_symbol, install_db_dir_lib, table_db_capability,
    table_db_mkdir_capability, table_db_read_capability, table_db_rmdir_capability,
    table_db_write_capability,
};

fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    sim_test_support::register_core_classes(&mut cx);
    cx
}

fn grant(cx: &mut sim_kernel::Cx, capabilities: &[sim_kernel::CapabilityName]) {
    for capability in capabilities {
        cx.grant(capability.clone());
    }
}

#[test]
fn db_dir_namespaced_subtables_are_isolated() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[
            table_db_capability(),
            table_db_read_capability(),
            table_db_write_capability(),
            table_db_mkdir_capability(),
            table_db_rmdir_capability(),
        ],
    );

    let root = install_db_dir_lib(&mut cx).unwrap();
    let root_dir = root.object().as_dir().unwrap();
    let root_table = root.object().as_table_impl().unwrap();
    let a = root_dir.mkdir(&mut cx, Symbol::new("a")).unwrap();
    let a_dir = a.object().as_dir().unwrap();
    let a_table = a.object().as_table_impl().unwrap();
    let nested = a_dir.mkdir(&mut cx, Symbol::new("b")).unwrap();
    let nested_table = nested.object().as_table_impl().unwrap();

    let root_value = cx.factory().string("root".to_owned()).unwrap();
    root_table
        .set(&mut cx, Symbol::new("x"), root_value)
        .unwrap();
    let child_value = cx.factory().string("child".to_owned()).unwrap();
    a_table.set(&mut cx, Symbol::new("x"), child_value).unwrap();
    let nested_value = cx.factory().string("deep".to_owned()).unwrap();
    nested_table
        .set(&mut cx, Symbol::new("y"), nested_value)
        .unwrap();

    assert_eq!(
        root_table
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("root".to_owned())
    );
    assert_eq!(
        a_table
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("child".to_owned())
    );
    assert_eq!(
        root_table
            .get(&mut cx, Symbol::new("y"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Nil
    );
    assert!(root_dir.is_dir(&mut cx, Symbol::new("a")).unwrap());
    assert_eq!(
        root_table.keys(&mut cx).unwrap(),
        vec![Symbol::new("a"), Symbol::new("x")]
    );
    assert_eq!(
        a_table.keys(&mut cx).unwrap(),
        vec![Symbol::new("b"), Symbol::new("x")]
    );

    root_dir.rmdir(&mut cx, Symbol::new("a")).unwrap();
    assert!(!root_dir.is_dir(&mut cx, Symbol::new("a")).unwrap());
    assert_eq!(
        root_table
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("root".to_owned())
    );
    assert_eq!(
        root_table
            .get(&mut cx, Symbol::new("a"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Nil
    );
}

#[test]
fn db_dir_dir_operations_are_capability_gated_and_names_are_checked() {
    let mut cx = cx();
    grant(&mut cx, &[table_db_capability()]);

    let root = install_db_dir_lib(&mut cx).unwrap();
    let root_dir = root.object().as_dir().unwrap();
    let root_table = root.object().as_table_impl().unwrap();
    let value = cx.factory().string("value".to_owned()).unwrap();

    assert!(matches!(
        root_table.get(&mut cx, Symbol::new("x")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        root_table.set(&mut cx, Symbol::new("x"), value),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        root_dir.mkdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));
    assert!(matches!(
        root_dir.rmdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { .. })
    ));

    grant(
        &mut cx,
        &[
            table_db_read_capability(),
            table_db_write_capability(),
            table_db_mkdir_capability(),
            table_db_rmdir_capability(),
        ],
    );

    let value = cx.factory().string("value".to_owned()).unwrap();
    root_table.set(&mut cx, Symbol::new("leaf"), value).unwrap();
    let err = root_dir.opendir(&mut cx, Symbol::new("leaf")).unwrap_err();
    assert!(err.to_string().contains("not a directory"));

    for illegal in ["", ".", "..", "a/b", "a\\b"] {
        let err = root_dir.mkdir(&mut cx, Symbol::new(illegal)).unwrap_err();
        assert!(err.to_string().contains("illegal name"));
    }
}

#[test]
fn db_dir_read_write_mkdir_and_rmdir_are_individually_capability_gated() {
    let mut cx = cx();
    grant(&mut cx, &[table_db_capability()]);

    let root = install_db_dir_lib(&mut cx).unwrap();
    let root_dir = root.object().as_dir().unwrap();
    let root_table = root.object().as_table_impl().unwrap();
    let value = cx.factory().string("value".to_owned()).unwrap();

    assert!(matches!(
        root_table.get(&mut cx, Symbol::new("x")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == table_db_read_capability()
    ));
    assert!(matches!(
        root_table.set(&mut cx, Symbol::new("x"), value),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == table_db_write_capability()
    ));
    assert!(matches!(
        root_dir.mkdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == table_db_mkdir_capability()
    ));
    assert!(matches!(
        root_dir.rmdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == table_db_rmdir_capability()
    ));
}

#[test]
fn db_dir_display_and_default_open_root_are_stable() {
    let mut cx = cx();
    grant(
        &mut cx,
        &[table_db_capability(), table_db_read_capability()],
    );

    let dir = DbDir::open();
    assert_eq!(dir.display(&mut cx).unwrap(), "table/db[/]");
    assert_eq!(dir.as_expr(&mut cx).unwrap(), Expr::Map(Vec::new()));
}

#[test]
fn db_dir_citizen_round_trips_as_descriptor_only() {
    let mut cx = cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    grant(
        &mut cx,
        &[
            table_db_capability(),
            table_db_read_capability(),
            table_db_mkdir_capability(),
        ],
    );
    let root = install_db_dir_lib(&mut cx).unwrap();
    let child = root
        .object()
        .as_dir()
        .unwrap()
        .mkdir(&mut cx, Symbol::new("child"))
        .unwrap();

    sim_citizen::check_value_fixture(&mut cx, child.clone()).unwrap();

    let ObjectEncoding::Constructor { args, .. } = child
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
    let decoded = cx.read_construct(&db_dir_class_symbol(), args).unwrap();
    let descriptor = decoded
        .object()
        .as_any()
        .downcast_ref::<DbDirDescriptor>()
        .expect("expected db descriptor");

    assert_eq!(descriptor.path, vec!["child".to_owned()]);
    assert!(decoded.object().as_table_impl().is_none());
    assert!(decoded.object().as_dir().is_none());
}

#[test]
fn db_dir_citizen_rejects_malformed_path() {
    let mut cx = cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let args = [
        Expr::Symbol(Symbol::new("v0")),
        Expr::List(vec![Expr::String("..".to_owned())]),
    ]
    .iter()
    .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
    .collect::<sim_kernel::Result<Vec<_>>>()
    .unwrap();

    let err = cx.read_construct(&db_dir_class_symbol(), args).unwrap_err();
    assert!(err.to_string().contains("illegal segment"));
}
