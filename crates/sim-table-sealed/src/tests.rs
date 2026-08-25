// conformance: sealed authenticated Table/Dir decorator and fail-closed bindings.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Expr, HandleSeed, Symbol, Value};
use sim_table_db::{
    install_db_dir_lib, table_db_capability, table_db_mkdir_capability, table_db_read_capability,
    table_db_rmdir_capability, table_db_write_capability,
};

use crate::{
    NONCE_LEN, NonceSource, SealedConfig, SealedError, SealedObject, SealedTable, SecretKey,
    blind::blind_key,
};

struct Keys(Mutex<Option<[u8; 32]>>);

impl Keys {
    fn new(key: [u8; 32]) -> Self {
        Self(Mutex::new(Some(key)))
    }
    fn replace(&self, key: Option<[u8; 32]>) {
        *self.0.lock().unwrap() = key;
    }
}

impl crate::KeyProvider for Keys {
    fn key(&self, _grant: &str) -> Option<SecretKey> {
        self.0
            .lock()
            .unwrap()
            .map(|key| SecretKey::new(format!("key-{}", key[0]), key, key.map(|byte| byte ^ 0xa5)))
    }
}

struct Nonces(Mutex<VecDeque<[u8; NONCE_LEN]>>);

impl Nonces {
    fn new(nonces: impl IntoIterator<Item = [u8; NONCE_LEN]>) -> Self {
        Self(Mutex::new(nonces.into_iter().collect()))
    }
}

impl NonceSource for Nonces {
    fn fill(&self, nonce: &mut [u8; NONCE_LEN]) -> Result<(), SealedError> {
        *nonce = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(SealedError::NonceBudgetExhausted)?;
        Ok(())
    }
}

fn cx() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(7),
    )
}

fn config(keys: Arc<Keys>, nonces: Arc<Nonces>, lane: &[u8], generation: u64) -> SealedConfig {
    SealedConfig {
        grant: "grant-7".into(),
        key_id: format!("key-{}", keys.0.lock().unwrap().unwrap()[0]),
        lane: lane.to_vec(),
        generation,
        max_plaintext_bytes: 4096,
        max_object_bytes: 8192,
        metadata_class: "portable-expression".into(),
        nonce_budget: 32,
        keys,
        nonces,
    }
}

fn wrap(cx: &Cx, backend: Value, config: SealedConfig) -> Value {
    cx.factory()
        .opaque(Arc::new(SealedTable::new(backend, config).unwrap()))
        .unwrap()
}

fn raw(backend: &Value, cx: &mut Cx, physical: &str) -> Vec<u8> {
    let value = backend
        .object()
        .as_table_impl()
        .unwrap()
        .get(cx, Symbol::new(physical))
        .unwrap();
    let Expr::Bytes(bytes) = value.object().as_expr(cx).unwrap() else {
        panic!("expected bytes")
    };
    bytes
}

fn physical(key: &[u8; 32], lane: &[u8], logical: &str) -> String {
    let blinding = key.map(|byte| byte ^ 0xa5);
    format!(
        "{}-{}",
        crate::blind::blind_lane(&blinding, lane),
        blind_key(&blinding, lane, logical)
    )
}

fn assert_auth_failure(result: sim_kernel::Result<Value>) {
    let text = result.unwrap_err().to_string();
    assert!(text.contains("authentication failed"), "{text}");
}

#[test]
fn round_trip_blinds_names_and_preserves_table_contract() {
    let mut cx = cx();
    let backend = cx.new_table(vec![]).unwrap();
    let key = [11; 32];
    let sealed = wrap(
        &cx,
        backend.clone(),
        config(
            Arc::new(Keys::new(key)),
            Arc::new(Nonces::new([[1; NONCE_LEN]])),
            b"finance",
            3,
        ),
    );
    let table = sealed.object().as_table_impl().unwrap();
    let private = cx.factory().string("private".into()).unwrap();
    table.set(&mut cx, Symbol::new("balance"), private).unwrap();

    assert_eq!(table.keys(&mut cx).unwrap(), vec![Symbol::new("balance")]);
    assert_eq!(
        table
            .get(&mut cx, Symbol::new("balance"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("private".into())
    );
    let backend_keys = backend
        .object()
        .as_table_impl()
        .unwrap()
        .keys(&mut cx)
        .unwrap();
    assert_eq!(
        backend_keys,
        vec![Symbol::new(physical(&key, b"finance", "balance"))]
    );
    assert!(!backend_keys[0].name.contains("balance"));
}

#[test]
fn wrong_key_revoked_grant_and_repeated_nonce_are_refused_without_secret_diagnostics() {
    let mut cx = cx();
    let backend = cx.new_table(vec![]).unwrap();
    let keys = Arc::new(Keys::new([21; 32]));
    let sealed = wrap(
        &cx,
        backend,
        config(
            keys.clone(),
            Arc::new(Nonces::new([[2; NONCE_LEN], [2; NONCE_LEN]])),
            b"private",
            1,
        ),
    );
    let table = sealed.object().as_table_impl().unwrap();
    let yes = cx.factory().bool(true).unwrap();
    table.set(&mut cx, Symbol::new("a"), yes).unwrap();
    let no = cx.factory().bool(false).unwrap();
    let repeated = table
        .set(&mut cx, Symbol::new("b"), no)
        .unwrap_err()
        .to_string();
    assert!(repeated.contains("nonce reuse refused"));

    keys.replace(Some([22; 32]));
    let wrong_key = table.keys(&mut cx).unwrap_err().to_string();
    assert!(wrong_key.contains("authentication failed"));
    keys.replace(None);
    let revoked = table
        .get(&mut cx, Symbol::new("a"))
        .unwrap_err()
        .to_string();
    assert!(revoked.contains("grant unavailable"));
    assert!(!revoked.contains("21"));
}

#[test]
fn moved_generation_metadata_tamper_and_swapped_ciphertext_fail_closed() {
    let mut cx = cx();
    let backend = cx.new_table(vec![]).unwrap();
    let key = [31; 32];
    let keys = Arc::new(Keys::new(key));
    let sealed = wrap(
        &cx,
        backend.clone(),
        config(
            keys.clone(),
            Arc::new(Nonces::new([[3; NONCE_LEN], [4; NONCE_LEN]])),
            b"lane-a",
            8,
        ),
    );
    let table = sealed.object().as_table_impl().unwrap();
    let one = cx.factory().string("one".into()).unwrap();
    table.set(&mut cx, Symbol::new("a"), one).unwrap();
    let two = cx.factory().string("two".into()).unwrap();
    table.set(&mut cx, Symbol::new("b"), two).unwrap();

    let lane_a_physical = physical(&key, b"lane-a", "a");
    let lane_b_physical = physical(&key, b"lane-b", "a");
    let moved_bytes = raw(&backend, &mut cx, &lane_a_physical);
    let moved_value = cx.factory().bytes(moved_bytes).unwrap();
    backend
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new(lane_b_physical), moved_value)
        .unwrap();
    let moved_lane = wrap(
        &cx,
        backend.clone(),
        config(keys.clone(), Arc::new(Nonces::new([])), b"lane-b", 8),
    );
    assert_auth_failure(
        moved_lane
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("a")),
    );
    let moved_generation = wrap(
        &cx,
        backend.clone(),
        config(keys.clone(), Arc::new(Nonces::new([])), b"lane-a", 9),
    );
    assert_auth_failure(
        moved_generation
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("a")),
    );
    let mut metadata_config = config(keys, Arc::new(Nonces::new([])), b"lane-a", 8);
    metadata_config.metadata_class = "other-class".into();
    let moved_metadata = wrap(&cx, backend.clone(), metadata_config);
    assert_auth_failure(
        moved_metadata
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("a")),
    );

    let pa = physical(&key, b"lane-a", "a");
    let pb = physical(&key, b"lane-a", "b");
    let a_bytes = raw(&backend, &mut cx, &pa);
    let b_bytes = raw(&backend, &mut cx, &pb);
    let backend_table = backend.object().as_table_impl().unwrap();
    let b_value = cx.factory().bytes(b_bytes).unwrap();
    backend_table
        .set(&mut cx, Symbol::new(pa.as_str()), b_value)
        .unwrap();
    let a_value = cx.factory().bytes(a_bytes).unwrap();
    backend_table
        .set(&mut cx, Symbol::new(pb.as_str()), a_value)
        .unwrap();
    assert_auth_failure(table.get(&mut cx, Symbol::new("a")));
    assert_auth_failure(table.get(&mut cx, Symbol::new("b")));
}

#[test]
fn forged_header_corruption_and_oversized_objects_are_refused_while_other_lane_is_stable() {
    let mut cx = cx();
    let backend = cx.new_table(vec![]).unwrap();
    let key = [41; 32];
    let keys = Arc::new(Keys::new(key));
    let lane_a = wrap(
        &cx,
        backend.clone(),
        config(
            keys.clone(),
            Arc::new(Nonces::new([[5; NONCE_LEN]])),
            b"a",
            1,
        ),
    );
    let lane_b = wrap(
        &cx,
        backend.clone(),
        config(keys, Arc::new(Nonces::new([[6; NONCE_LEN]])), b"b", 1),
    );
    let yes = cx.factory().bool(true).unwrap();
    lane_a
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("x"), yes)
        .unwrap();
    let no = cx.factory().bool(false).unwrap();
    lane_b
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("x"), no)
        .unwrap();
    let pa = physical(&key, b"a", "x");
    let pb = physical(&key, b"b", "x");
    let stable = raw(&backend, &mut cx, &pb);

    for index in [0usize, 1, 17, 18] {
        let mut corrupt = raw(&backend, &mut cx, &pa);
        corrupt[index] ^= 0x40;
        let corrupt_value = cx.factory().bytes(corrupt).unwrap();
        backend
            .object()
            .as_table_impl()
            .unwrap()
            .set(&mut cx, Symbol::new(pa.as_str()), corrupt_value)
            .unwrap();
        assert_auth_failure(
            lane_a
                .object()
                .as_table_impl()
                .unwrap()
                .get(&mut cx, Symbol::new("x")),
        );
    }
    assert_eq!(raw(&backend, &mut cx, &pb), stable);
    assert_eq!(
        lane_b
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Bool(false)
    );
    lane_a
        .object()
        .as_table_impl()
        .unwrap()
        .clear(&mut cx)
        .unwrap();
    assert_eq!(raw(&backend, &mut cx, &pb), stable);
    assert_eq!(
        lane_b
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::Bool(false)
    );

    assert_eq!(
        SealedObject::decode(&vec![0; 9000], 8192),
        Err(SealedError::Oversized)
    );
}

#[test]
fn persistent_dir_is_decorated_only_through_table_and_dir_contracts() {
    let mut cx = cx();
    for capability in [
        table_db_capability(),
        table_db_read_capability(),
        table_db_write_capability(),
        table_db_mkdir_capability(),
        table_db_rmdir_capability(),
    ] {
        cx.grant(capability);
    }
    let backend = install_db_dir_lib(&mut cx).unwrap();
    let sealed = wrap(
        &cx,
        backend,
        config(
            Arc::new(Keys::new([51; 32])),
            Arc::new(Nonces::new([[7; NONCE_LEN]])),
            b"root",
            1,
        ),
    );
    let child = sealed
        .object()
        .as_dir()
        .unwrap()
        .mkdir(&mut cx, Symbol::new("child"))
        .unwrap();
    let value = cx.factory().string("value".into()).unwrap();
    child
        .object()
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("secret"), value)
        .unwrap();
    let reopened = sealed
        .object()
        .as_dir()
        .unwrap()
        .opendir(&mut cx, Symbol::new("child"))
        .unwrap()
        .unwrap();
    assert_eq!(
        reopened
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("secret"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("value".into())
    );
    let removed = sealed
        .object()
        .as_dir()
        .unwrap()
        .rmdir(&mut cx, Symbol::new("child"))
        .unwrap();
    assert_eq!(removed.object().as_expr(&mut cx).unwrap(), Expr::Nil);
    assert!(
        sealed
            .object()
            .as_dir()
            .unwrap()
            .opendir(&mut cx, Symbol::new("child"))
            .unwrap()
            .is_none()
    );
}
