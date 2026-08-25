//! Contract-preserving `Table`/`Dir` authenticated-encryption decorator.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use sim_kernel::{
    Cx, Dir, Error, Expr, Factory, Object, Result, Symbol, Table, Value, id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
};
use zeroize::Zeroizing;

use crate::{
    Binding, NONCE_LEN, SUITE_CHACHA20_POLY1305, SealedError, SealedObject,
    blind::{blind_key, blind_lane},
    codec,
};

/// Bounded, purpose-separated AEAD and name-blinding keys, zeroized on drop.
pub struct SecretKey {
    id: String,
    aead: Zeroizing<[u8; 32]>,
    blinding: Zeroizing<[u8; 32]>,
}

impl SecretKey {
    /// Copy purpose-separated key material supplied by a host-owned secret service.
    pub fn new(id: impl Into<String>, aead: [u8; 32], blinding: [u8; 32]) -> Self {
        Self {
            id: id.into(),
            aead: Zeroizing::new(aead),
            blinding: Zeroizing::new(blinding),
        }
    }
    fn aead(&self) -> &[u8; 32] {
        &self.aead
    }
    fn blinding(&self) -> &[u8; 32] {
        &self.blinding
    }
    fn id(&self) -> &str {
        &self.id
    }
}

/// Read-only, revocable key-grant interface.
pub trait KeyProvider: Send + Sync {
    /// Return a fresh bounded, purpose-separated copy for `grant`, or `None` when absent/revoked.
    fn key(&self, grant: &str) -> Option<SecretKey>;
}

/// Injected cryptographically secure nonce interface.
pub trait NonceSource: Send + Sync {
    /// Fill one nonce. Implementations must be unpredictable or uniqueness-proven.
    fn fill(&self, nonce: &mut [u8; NONCE_LEN]) -> std::result::Result<(), SealedError>;
}

/// Immutable decorator policy and binding identity.
#[derive(Clone)]
pub struct SealedConfig {
    /// Opaque key-provider grant id (never key material).
    pub grant: String,
    /// Opaque key identity expected for this lane generation.
    pub key_id: String,
    /// Opaque lane identity, authenticated and used for key-name separation.
    pub lane: Vec<u8>,
    /// Restore/rotation generation, authenticated on every object.
    pub generation: u64,
    /// Maximum plaintext payload size.
    pub max_plaintext_bytes: usize,
    /// Maximum encoded sealed-object size.
    pub max_object_bytes: usize,
    /// Stable metadata class authenticated for every value in this lane.
    pub metadata_class: String,
    /// Maximum writes per decorator instance, bounding nonce tracking memory.
    pub nonce_budget: usize,
    /// Host-owned revocable key provider.
    pub keys: Arc<dyn KeyProvider>,
    /// Host-owned secure randomness source.
    pub nonces: Arc<dyn NonceSource>,
}

/// A sealed view over an existing table or directory backend.
#[derive(Clone)]
pub struct SealedTable {
    backend: Value,
    config: SealedConfig,
    used_nonces: Arc<Mutex<HashSet<[u8; NONCE_LEN]>>>,
}

impl SealedTable {
    /// Wrap `backend` without changing or owning its storage semantics.
    pub fn new(backend: Value, config: SealedConfig) -> Result<Self> {
        if backend.object().as_table_impl().is_none() {
            return Err(Error::Eval(
                "table/sealed: backend must implement Table".into(),
            ));
        }
        if config.max_plaintext_bytes == 0
            || config.max_object_bytes < config.max_plaintext_bytes + 34
            || config.metadata_class.is_empty()
            || config.key_id.is_empty()
            || config.nonce_budget == 0
        {
            return Err(Error::Eval("table/sealed: invalid size policy".into()));
        }
        Ok(Self {
            backend,
            config,
            used_nonces: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    fn table(&self) -> &dyn Table {
        self.backend
            .object()
            .as_table_impl()
            .expect("validated table")
    }
    fn dir(&self) -> Option<&dyn Dir> {
        self.backend.object().as_dir()
    }
    fn key(&self) -> Result<SecretKey> {
        let key = self
            .config
            .keys
            .key(&self.config.grant)
            .ok_or_else(|| failure(SealedError::GrantUnavailable))?;
        if key.id() != self.config.key_id {
            return Err(failure(SealedError::Authentication));
        }
        Ok(key)
    }
    fn physical(&self, key: &SecretKey, logical: &Symbol) -> Symbol {
        Symbol::new(format!(
            "{}-{}",
            self.lane_prefix(key),
            blind_key(key.blinding(), &self.config.lane, &logical.name)
        ))
    }
    fn lane_prefix(&self, key: &SecretKey) -> String {
        blind_lane(key.blinding(), &self.config.lane)
    }
    fn belongs_to_lane(&self, key: &SecretKey, physical: &Symbol) -> bool {
        physical
            .name
            .strip_prefix(&self.lane_prefix(key))
            .is_some_and(|rest| rest.starts_with('-'))
    }
    fn binding(&self, physical: &Symbol) -> Binding {
        Binding {
            lane: self.config.lane.clone(),
            generation: self.config.generation,
            physical_key: physical.name.to_string(),
            metadata_class: self.config.metadata_class.clone(),
        }
    }

    fn seal(
        &self,
        logical: &Symbol,
        physical: &Symbol,
        value: &Value,
        cx: &mut Cx,
        key: &SecretKey,
    ) -> Result<Value> {
        let expr = value.object().as_expr(cx)?;
        let mut plaintext = codec::encode(&logical.name, &expr, self.config.max_plaintext_bytes)
            .map_err(failure)?;
        let mut nonce_bytes = [0; NONCE_LEN];
        self.config.nonces.fill(&mut nonce_bytes).map_err(failure)?;
        let mut used = self
            .used_nonces
            .lock()
            .map_err(|_| failure(SealedError::NonceReuse))?;
        if used.len() >= self.config.nonce_budget {
            return Err(failure(SealedError::NonceBudgetExhausted));
        }
        if !used.insert(nonce_bytes) {
            return Err(failure(SealedError::NonceReuse));
        }
        drop(used);
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, key.aead())
            .map_err(|_| failure(SealedError::Authentication))?;
        let tag = LessSafeKey::new(unbound)
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(self.binding(physical).aad()),
                &mut plaintext,
            )
            .map_err(|_| failure(SealedError::Authentication))?;
        plaintext.extend_from_slice(tag.as_ref());
        let object = SealedObject {
            suite: SUITE_CHACHA20_POLY1305,
            nonce: nonce_bytes,
            ciphertext_and_tag: plaintext.to_vec(),
        };
        let bytes = object.encode().map_err(failure)?;
        if bytes.len() > self.config.max_object_bytes {
            return Err(failure(SealedError::Oversized));
        }
        cx.factory().bytes(bytes)
    }

    fn open(
        &self,
        logical: Option<&Symbol>,
        physical: &Symbol,
        stored: Value,
        cx: &mut Cx,
        key: &SecretKey,
    ) -> Result<(Symbol, Value)> {
        let expr = stored.object().as_expr(cx)?;
        let Expr::Bytes(bytes) = expr else {
            return Err(failure(SealedError::Authentication));
        };
        let object = SealedObject::decode(&bytes, self.config.max_object_bytes).map_err(failure)?;
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, key.aead())
            .map_err(|_| failure(SealedError::Authentication))?;
        let mut ciphertext = Zeroizing::new(object.ciphertext_and_tag);
        let plaintext = LessSafeKey::new(unbound)
            .open_in_place(
                Nonce::assume_unique_for_key(object.nonce),
                Aad::from(self.binding(physical).aad()),
                &mut ciphertext,
            )
            .map_err(|_| failure(SealedError::Authentication))?;
        let (decoded_key, decoded_expr) =
            codec::decode(plaintext, self.config.max_plaintext_bytes).map_err(failure)?;
        let decoded = Symbol::new(decoded_key);
        if logical.is_some_and(|expected| *expected != decoded)
            || self.physical(key, &decoded) != *physical
        {
            return Err(failure(SealedError::Authentication));
        }
        Ok((decoded, cx.factory().expr(decoded_expr)?))
    }

    fn child(&self, backend: Value, name: &Symbol) -> Result<Value> {
        let mut config = self.config.clone();
        config
            .lane
            .extend_from_slice(&(name.name.len() as u64).to_be_bytes());
        config.lane.extend_from_slice(name.name.as_bytes());
        let child = Self {
            backend,
            config,
            used_nonces: Arc::clone(&self.used_nonces),
        };
        sim_kernel::DefaultFactory.opaque(Arc::new(child))
    }
}

impl Object for SealedTable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("table/sealed[redacted]".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for SealedTable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = Symbol::qualified("core", "Table");
        cx.registry()
            .class_by_symbol(&symbol)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol))
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_dir(&self) -> Option<&dyn Dir> {
        self.dir().map(|_| self as &dyn Dir)
    }
}

impl Table for SealedTable {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "sealed")
    }
    fn get(&self, cx: &mut Cx, logical: Symbol) -> Result<Value> {
        let key = self.key()?;
        let physical = self.physical(&key, &logical);
        if !self.table().has(cx, physical.clone())? {
            return cx.factory().nil();
        }
        let stored = self.table().get(cx, physical.clone())?;
        self.open(Some(&logical), &physical, stored, cx, &key)
            .map(|(_, value)| value)
    }
    fn set(&self, cx: &mut Cx, logical: Symbol, value: Value) -> Result<()> {
        let key = self.key()?;
        let physical = self.physical(&key, &logical);
        let sealed = self.seal(&logical, &physical, &value, cx, &key)?;
        self.table().set(cx, physical, sealed)
    }
    fn has(&self, cx: &mut Cx, logical: Symbol) -> Result<bool> {
        let key = self.key()?;
        let physical = self.physical(&key, &logical);
        if !self.table().has(cx, physical.clone())? {
            return Ok(false);
        }
        self.open(
            Some(&logical),
            &physical.clone(),
            self.table().get(cx, physical)?,
            cx,
            &key,
        )
        .map(|_| true)
    }
    fn del(&self, cx: &mut Cx, logical: Symbol) -> Result<Value> {
        let prior = self.get(cx, logical.clone())?;
        let key = self.key()?;
        self.table().del(cx, self.physical(&key, &logical))?;
        Ok(prior)
    }
    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        let key = self.key()?;
        self.table()
            .keys(cx)?
            .into_iter()
            .filter(|physical| self.belongs_to_lane(&key, physical))
            .map(|physical| {
                let stored = self.table().get(cx, physical.clone())?;
                self.open(None, &physical, stored, cx, &key)
                    .map(|(logical, _)| logical)
            })
            .collect()
    }
    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let key = self.key()?;
        self.table()
            .keys(cx)?
            .into_iter()
            .filter(|physical| self.belongs_to_lane(&key, physical))
            .map(|physical| {
                let stored = self.table().get(cx, physical.clone())?;
                self.open(None, &physical, stored, cx, &key)
            })
            .collect()
    }
    fn len(&self, cx: &mut Cx) -> Result<usize> {
        self.keys(cx).map(|keys| keys.len())
    }
    fn clear(&self, cx: &mut Cx) -> Result<()> {
        let key = self.key()?;
        let physical_keys: Vec<_> = self
            .table()
            .keys(cx)?
            .into_iter()
            .filter(|physical| self.belongs_to_lane(&key, physical))
            .collect();
        for physical in physical_keys {
            self.table().del(cx, physical)?;
        }
        Ok(())
    }
}

impl Dir for SealedTable {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        let key = self.key()?;
        let physical = self.physical(&key, &name);
        let backend = self
            .dir()
            .ok_or_else(|| Error::Eval("table/sealed: backend does not implement Dir".into()))?
            .mkdir(cx, physical)?;
        self.child(backend, &name)
    }
    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        let key = self.key()?;
        let physical = self.physical(&key, &name);
        self.dir()
            .ok_or_else(|| Error::Eval("table/sealed: backend does not implement Dir".into()))?
            .opendir(cx, physical)?
            .map(|backend| self.child(backend, &name))
            .transpose()
    }
    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        let key = self.key()?;
        let physical = self.physical(&key, &name);
        self.dir()
            .ok_or_else(|| Error::Eval("table/sealed: backend does not implement Dir".into()))?
            .rmdir(cx, physical)
    }
    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        let key = self.key()?;
        let physical = self.physical(&key, &name);
        self.dir()
            .ok_or_else(|| Error::Eval("table/sealed: backend does not implement Dir".into()))?
            .is_dir(cx, physical)
    }
}

fn failure(error: SealedError) -> Error {
    Error::Eval(format!("table/sealed: {error}"))
}
