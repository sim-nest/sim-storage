use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sim_kernel::{
    Cx, DefaultFactory, Expr, NoopEvalPolicy, ObjectCompat, ObjectEncode, ObjectEncoding, Symbol,
    Table, read_construct_capability,
};

use crate::{
    LazyBackend, LazyTable, LazyTableDescriptor, ValueLoader, install_lazy_table_lib,
    lazy_table_class_symbol,
};

fn test_cx() -> Cx {
    Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory))
}

#[test]
fn lazy_value_is_forced_once() {
    let mut cx = test_cx();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let loader: ValueLoader = Arc::new(move |cx: &mut Cx| {
        counter.fetch_add(1, Ordering::SeqCst);
        cx.factory().bool(true)
    });
    let table = LazyTable::with_loaders(vec![(Symbol::new("x"), loader)]);

    let first = table.get(&mut cx, Symbol::new("x")).unwrap();
    let second = table.get(&mut cx, Symbol::new("x")).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first, second);
}

#[test]
fn metadata_methods_do_not_force_but_entries_and_as_expr_do() {
    let mut cx = test_cx();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let loader: ValueLoader = Arc::new(move |cx: &mut Cx| {
        counter.fetch_add(1, Ordering::SeqCst);
        cx.factory().string("loaded".to_owned())
    });
    let table = LazyTable::with_loaders(vec![(Symbol::new("x"), loader)]);

    assert!(table.has(&mut cx, Symbol::new("x")).unwrap());
    assert_eq!(table.len(&mut cx).unwrap(), 1);
    assert_eq!(table.keys(&mut cx).unwrap(), vec![Symbol::new("x")]);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let entries = table.entries(&mut cx).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let expr = table.as_expr(&mut cx).unwrap();
    assert_eq!(
        expr,
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("x")),
            Expr::String("loaded".to_owned()),
        )])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn eager_backend_entries_are_pre_cached() {
    let mut cx = test_cx();
    let value = cx.factory().string("cached".to_owned()).unwrap();
    let table = LazyTable::with_entries(vec![(Symbol::new("x"), value.clone())]);

    assert_eq!(table.get(&mut cx, Symbol::new("x")).unwrap(), value);
    assert_eq!(
        table.as_expr(&mut cx).unwrap(),
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("x")),
            Expr::String("cached".to_owned()),
        )])
    );
}

#[test]
fn loader_errors_are_memoized_too() {
    let mut cx = test_cx();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let loader: ValueLoader = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Err(sim_kernel::Error::Eval("boom".to_owned()))
    });
    let table = LazyTable::with_loaders(vec![(Symbol::new("x"), loader)]);

    assert!(matches!(
        table.get(&mut cx, Symbol::new("x")),
        Err(sim_kernel::Error::Eval(message)) if message == "boom"
    ));
    assert!(matches!(
        table.get(&mut cx, Symbol::new("x")),
        Err(sim_kernel::Error::Eval(message)) if message == "boom"
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn install_registers_lazy_backend() {
    let mut cx = test_cx();
    install_lazy_table_lib(&mut cx).unwrap();
    cx.table_registry_mut().set_active("lazy").unwrap();

    assert_eq!(cx.table_registry().active(), "lazy");
    let name = <LazyBackend as sim_kernel::TableBackend>::name(&LazyBackend);
    assert_eq!(name, "lazy");
}

#[test]
fn lazy_table_citizen_round_trips_as_descriptor() {
    let mut cx = test_cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let value = cx.factory().string("cached".to_owned()).unwrap();
    let table = LazyTable::with_entries(vec![(Symbol::new("x"), value)]);
    let original = cx.factory().opaque(Arc::new(table)).unwrap();

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
    let decoded = cx.read_construct(&lazy_table_class_symbol(), args).unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<LazyTableDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_table_impl().is_none());
}

#[test]
fn poisoned_lock_returns_err_instead_of_panicking() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let table = LazyTable::new();
    // A loader that panics when forced. `del` forces the entry while still
    // holding the write guard, so the panic poisons the backing lock.
    let loader: ValueLoader = Arc::new(|_: &mut Cx| panic!("loader exploded"));
    table.put_lazy(Symbol::new("boom"), loader);

    let mut cx = test_cx();
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _ = table.del(&mut cx, Symbol::new("boom"));
    }));
    std::panic::set_hook(previous_hook);
    assert!(
        panicked.is_err(),
        "forcing the loader must panic and poison"
    );

    // With the lock now poisoned, a later operation must surface a clean error
    // rather than cascade the panic.
    let mut cx = test_cx();
    let err = table
        .len(&mut cx)
        .expect_err("operation on a poisoned lock must return Err, not panic");
    assert!(
        err.to_string().contains("poisoned"),
        "expected a poisoned-lock error, got: {err}"
    );
}

#[test]
fn lazy_table_citizen_encoding_fails_closed_on_loader_error() {
    let mut cx = test_cx();
    let loader: ValueLoader = Arc::new(|_| Err(sim_kernel::Error::Eval("denied".to_owned())));
    let table = LazyTable::with_loaders(vec![(Symbol::new("x"), loader)]);

    let err = table
        .object_encoding(&mut cx)
        .expect_err("loader error must stop descriptor encoding");
    assert!(err.to_string().contains("denied"));
}
