use std::sync::Arc;

// conformance: mounted namespaces compose Table and Dir backends without flattening them.

use sim_kernel::{Args, DefaultFactory, Dir, EagerPolicy, Expr, Symbol, Table};
use sim_table_core::TablePath;
use sim_table_db::{
    DbDir, table_db_capability, table_db_mkdir_capability, table_db_read_capability,
    table_db_rmdir_capability, table_db_write_capability,
};

use crate::{
    MountKind, MountedDir, install_mount_dir_lib, mount_create_symbol, mount_dir_symbol,
    mount_inspect_symbol, mount_table_symbol, mount_unmount_symbol, table_mount_capability,
};

fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    sim_test_support::register_core_classes(&mut cx);
    grant(
        &mut cx,
        &[
            table_mount_capability(),
            table_db_capability(),
            table_db_read_capability(),
            table_db_write_capability(),
            table_db_mkdir_capability(),
            table_db_rmdir_capability(),
        ],
    );
    cx
}

fn grant(cx: &mut sim_kernel::Cx, capabilities: &[sim_kernel::CapabilityName]) {
    for capability in capabilities {
        cx.grant(capability.clone());
    }
}

fn db_value(cx: &mut sim_kernel::Cx) -> sim_kernel::Value {
    cx.factory().opaque(Arc::new(DbDir::open())).unwrap()
}

fn table(cx: &mut sim_kernel::Cx, entries: &[(&str, sim_kernel::Value)]) -> sim_kernel::Value {
    cx.factory()
        .table(
            entries
                .iter()
                .map(|(key, value)| (Symbol::new(*key), value.clone()))
                .collect(),
        )
        .unwrap()
}

fn string(cx: &mut sim_kernel::Cx, value: &str) -> sim_kernel::Value {
    cx.factory().string(value.to_owned()).unwrap()
}

fn absolute(path: &str) -> TablePath {
    TablePath::parse_absolute(path).unwrap()
}

fn expr(cx: &mut sim_kernel::Cx, value: sim_kernel::Value) -> Expr {
    value.object().as_expr(cx).unwrap()
}

#[test]
fn mounted_dir_routes_nested_dirs_by_longest_prefix() {
    let mut cx = cx();
    let root = db_value(&mut cx);
    let remote = db_value(&mut cx);
    let nested = db_value(&mut cx);
    let namespace = MountedDir::new(root).unwrap();
    namespace
        .mount_dir(&mut cx, absolute("/remote"), remote.clone())
        .unwrap();
    namespace
        .mount_dir(&mut cx, absolute("/remote/nested"), nested.clone())
        .unwrap();

    let remote_value = string(&mut cx, "remote");
    remote
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("from-remote"), remote_value)
        .unwrap();
    let nested_value = string(&mut cx, "nested");
    nested
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("from-nested"), nested_value)
        .unwrap();

    let remote_view = namespace
        .opendir(&mut cx, Symbol::new("remote"))
        .unwrap()
        .unwrap();
    let remote_table = remote_view.object().as_table_impl().unwrap();
    let remote_result = remote_table
        .get(&mut cx, Symbol::new("from-remote"))
        .unwrap();
    assert_eq!(
        expr(&mut cx, remote_result),
        Expr::String("remote".to_owned())
    );
    assert!(remote_table.has(&mut cx, Symbol::new("nested")).unwrap());

    let nested_view = remote_view
        .object()
        .as_dir()
        .unwrap()
        .opendir(&mut cx, Symbol::new("nested"))
        .unwrap()
        .unwrap();
    let nested_result = nested_view
        .object()
        .as_table_impl()
        .unwrap()
        .get(&mut cx, Symbol::new("from-nested"))
        .unwrap();
    assert_eq!(
        expr(&mut cx, nested_result),
        Expr::String("nested".to_owned())
    );
}

#[test]
fn table_mounts_are_visible_leaves_and_not_directories() {
    let mut cx = cx();
    let namespace = MountedDir::new(db_value(&mut cx)).unwrap();
    let answer = cx.factory().bool(true).unwrap();
    let leaf = table(&mut cx, &[("answer", answer)]);
    namespace
        .mount_table(&mut cx, absolute("/lookup"), leaf.clone())
        .unwrap();

    assert!(namespace.has(&mut cx, Symbol::new("lookup")).unwrap());
    assert_eq!(namespace.get(&mut cx, Symbol::new("lookup")).unwrap(), leaf);
    assert!(matches!(
        namespace.opendir(&mut cx, Symbol::new("lookup")),
        Err(sim_kernel::Error::Eval(message)) if message.contains("table mount")
    ));
    assert_eq!(namespace.entries(&mut cx).unwrap().len(), 1);
}

#[test]
fn deeper_table_mounts_create_virtual_parent_dirs() {
    let mut cx = cx();
    let namespace = MountedDir::new(db_value(&mut cx)).unwrap();
    let value = string(&mut cx, "leaf");
    let leaf = table(&mut cx, &[("value", value)]);
    namespace
        .mount_table(&mut cx, absolute("/virtual/leaf"), leaf.clone())
        .unwrap();

    assert!(namespace.has(&mut cx, Symbol::new("virtual")).unwrap());
    assert!(namespace.is_dir(&mut cx, Symbol::new("virtual")).unwrap());
    assert!(namespace.entries(&mut cx).unwrap().is_empty());

    let virtual_dir = namespace
        .opendir(&mut cx, Symbol::new("virtual"))
        .unwrap()
        .unwrap();
    let virtual_table = virtual_dir.object().as_table_impl().unwrap();
    assert!(virtual_table.has(&mut cx, Symbol::new("leaf")).unwrap());
    assert_eq!(
        virtual_table.get(&mut cx, Symbol::new("leaf")).unwrap(),
        leaf
    );
}

#[test]
fn mount_conflicts_fail_closed_around_existing_nodes_and_mounts() {
    let mut cx = cx();
    let root = db_value(&mut cx);
    let namespace = MountedDir::new(root.clone()).unwrap();
    let existing = string(&mut cx, "root");
    root.object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("existing"), existing)
        .unwrap();

    let shadow = table(&mut cx, &[]);
    assert!(matches!(
        namespace.mount_table(&mut cx, absolute("/existing"), shadow),
        Err(sim_kernel::Error::Eval(message)) if message.contains("shadows")
    ));

    root.object()
        .as_dir()
        .unwrap()
        .mkdir(&mut cx, Symbol::new("kept"))
        .unwrap();
    let leaf = table(&mut cx, &[]);
    namespace
        .mount_table(&mut cx, absolute("/kept/leaf"), leaf)
        .unwrap();
    assert!(matches!(
        namespace.rmdir(&mut cx, Symbol::new("kept")),
        Err(sim_kernel::Error::Eval(message)) if message.contains("mount exists")
    ));
    assert!(matches!(
        namespace.del(&mut cx, Symbol::new("kept")),
        Err(sim_kernel::Error::Eval(message)) if message.contains("mount exists")
    ));
}

#[test]
fn unmount_and_root_protection_are_explicit() {
    let mut cx = cx();
    let namespace = MountedDir::new(db_value(&mut cx)).unwrap();
    let root_target = db_value(&mut cx);
    assert!(matches!(
        namespace.mount_dir(&mut cx, absolute("/"), root_target),
        Err(sim_kernel::Error::Eval(message)) if message.contains("cannot mount root")
    ));
    let leaf = table(&mut cx, &[]);
    namespace
        .mount_table(&mut cx, absolute("/leaf"), leaf)
        .unwrap();
    assert_eq!(namespace.inspect().unwrap()[0].kind, MountKind::Table);
    assert!(
        namespace
            .unmount(&mut cx, &absolute("/leaf"))
            .unwrap()
            .is_some()
    );
    assert!(!namespace.has(&mut cx, Symbol::new("leaf")).unwrap());
}

#[test]
fn mounted_namespace_observes_backend_state_and_capabilities() {
    let mut authorized = cx();
    let remote = db_value(&mut authorized);
    let namespace = MountedDir::new(db_value(&mut authorized)).unwrap();
    namespace
        .mount_dir(&mut authorized, absolute("/remote"), remote.clone())
        .unwrap();

    let remote_view = namespace
        .opendir(&mut authorized, Symbol::new("remote"))
        .unwrap()
        .unwrap();
    let epoch = string(&mut authorized, "after-mount");
    remote
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut authorized, Symbol::new("epoch"), epoch)
        .unwrap();
    let epoch_result = remote_view
        .object()
        .as_table_impl()
        .unwrap()
        .get(&mut authorized, Symbol::new("epoch"))
        .unwrap();
    assert_eq!(
        expr(&mut authorized, epoch_result),
        Expr::String("after-mount".to_owned())
    );

    let mut denied = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    denied.grant(table_mount_capability());
    let denied_namespace = MountedDir::new(db_value(&mut denied)).unwrap();
    assert!(matches!(
        denied_namespace.get(&mut denied, Symbol::new("x")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == table_db_read_capability()
    ));
}

#[test]
fn exported_functions_create_mount_inspect_and_unmount() {
    let mut cx = cx();
    install_mount_dir_lib(&mut cx).unwrap();
    let root = db_value(&mut cx);
    let namespace = cx
        .call_function(&mount_create_symbol(), Args::new(vec![root]))
        .unwrap();
    let v = string(&mut cx, "v");
    let table = table(&mut cx, &[("k", v)]);
    let path = cx.factory().string("/lookup".to_owned()).unwrap();
    cx.call_function(
        &mount_table_symbol(),
        Args::new(vec![namespace.clone(), path.clone(), table.clone()]),
    )
    .unwrap();
    let inspected = cx
        .call_function(&mount_inspect_symbol(), Args::new(vec![namespace.clone()]))
        .unwrap();
    assert!(matches!(
        expr(&mut cx, inspected),
        Expr::List(rows) if !rows.is_empty()
    ));
    assert_eq!(
        namespace
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("lookup"))
            .unwrap(),
        table
    );
    cx.call_function(
        &mount_unmount_symbol(),
        Args::new(vec![namespace.clone(), path]),
    )
    .unwrap();
    assert!(
        !namespace
            .object()
            .as_table_impl()
            .unwrap()
            .has(&mut cx, Symbol::new("lookup"))
            .unwrap()
    );

    let target_dir = db_value(&mut cx);
    let dir_path = cx.factory().string("/remote".to_owned()).unwrap();
    cx.call_function(
        &mount_dir_symbol(),
        Args::new(vec![namespace, dir_path, target_dir]),
    )
    .unwrap();
}
