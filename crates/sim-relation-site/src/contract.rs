use std::{fmt, time::Duration};

use sim_kernel::{CapabilityName, Datum, Symbol};
use sim_relation_core::{Cell, Row, RowType};
use sim_relation_migrate::CheckedProgram;
use sim_relation_plan::{CheckedMutation, CheckedQuery};

/// Provider-neutral storage authority passed to a relation driver.
///
/// Paths are deliberately absent: the capsule resolves the opaque reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageLocator {
    /// A private, connection-scoped database.
    Memory,
    /// A preopened storage reference with explicit write authority.
    Preopened {
        /// Stable preopened reference resolved only by the capsule.
        reference: Symbol,
        /// Authority granted for this connection.
        access: StorageAccess,
    },
}

/// Authority carried by a preopened storage reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageAccess {
    /// Reads are admitted and mutations fail closed.
    ReadOnly,
    /// Reads and admitted mutations are allowed.
    ReadWrite,
}

impl StorageLocator {
    /// Decodes the closed public locator grammar.
    pub fn from_datum(value: &Datum) -> Result<Self, SiteError> {
        let Datum::Node { tag, fields } = value else {
            return Err(SiteError::Locator);
        };
        if tag == &Symbol::qualified("relation", "memory") && fields.is_empty() {
            return Ok(Self::Memory);
        }
        if tag != &Symbol::qualified("relation", "preopened") {
            return Err(SiteError::Locator);
        }
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == &Symbol::new(name))
                .map(|(_, value)| value)
        };
        if fields.len() != 2 {
            return Err(SiteError::Locator);
        }
        let Some(Datum::Symbol(reference)) = field("ref") else {
            return Err(SiteError::Locator);
        };
        let access = match field("access") {
            Some(Datum::Symbol(value)) if value == &Symbol::new("read-only") => {
                StorageAccess::ReadOnly
            }
            Some(Datum::Symbol(value)) if value == &Symbol::new("read-write") => {
                StorageAccess::ReadWrite
            }
            _ => return Err(SiteError::Locator),
        };
        Ok(Self::Preopened {
            reference: reference.clone(),
            access,
        })
    }
}

/// Declared provider identity checked before a driver can be installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriverManifest {
    /// Exact kernel site export.
    pub site: Symbol,
    /// Exact provider behavior identity.
    pub provider: Symbol,
}
impl DriverManifest {
    /// Admits only the canonical SQLite realization and rejects aliases.
    pub fn sqlite(site: Symbol, provider: Symbol) -> Result<Self, SiteError> {
        if site != Symbol::qualified("relation/site", "sqlite")
            || provider != Symbol::qualified("relation/provider", "sqlite")
        {
            return Err(SiteError::Registration);
        }
        Ok(Self { site, provider })
    }
}

/// A loaded site name paired with a provider-opaque locator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationPlacement {
    pub(crate) site: Symbol,
    pub(crate) locator: Datum,
}
impl RelationPlacement {
    /// Names a loaded relation site and preserves the locator as ordinary data.
    pub fn new(site: Symbol, locator: Datum) -> Self {
        Self { site, locator }
    }
    /// Loaded site symbol.
    pub fn site(&self) -> &Symbol {
        &self.site
    }
    /// Opaque locator, interpreted only by the selected driver.
    pub fn locator(&self) -> &Datum {
        &self.locator
    }
}

/// Mandatory execution maxima and optional time/buffering bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum emitted rows.
    pub rows: u64,
    /// Maximum emitted cells.
    pub cells: u64,
    /// Maximum logical datum bytes.
    pub bytes: u64,
    /// Maximum provider work units.
    pub work: u64,
    /// Optional elapsed deadline.
    pub deadline: Option<Duration>,
    /// Optional maximum provider buffering.
    pub buffer_bytes: Option<u64>,
}
impl Limits {
    /// Constructs limits, rejecting every zero bound.
    pub fn new(rows: u64, cells: u64, bytes: u64, work: u64) -> Result<Self, SiteError> {
        if [rows, cells, bytes, work].contains(&0) {
            return Err(SiteError::InvalidLimits);
        }
        Ok(Self {
            rows,
            cells,
            bytes,
            work,
            deadline: None,
            buffer_bytes: None,
        })
    }
}

/// Ordered, typed parameter values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings(Row);
impl Bindings {
    /// Checks arity, domains, and nullability against a plan's parameter type.
    pub fn new(
        expected: &RowType,
        cells: impl IntoIterator<Item = Cell>,
    ) -> Result<Self, SiteError> {
        Row::new(expected.clone(), cells)
            .map(Self)
            .map_err(|e| SiteError::Bindings(e.to_string()))
    }
    /// Checked row supplied to providers.
    pub fn row(&self) -> &Row {
        &self.0
    }
}

/// Push target for query and returning rows.
pub trait RowSink {
    /// Accepts one already type-checked row.
    fn push(&mut self, row: Row) -> Result<(), SiteError>;
}

/// Provider-reported bounded work and redaction-safe facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderStats {
    /// Provider work units consumed.
    pub work: u64,
    /// Rows affected by a mutation or migration.
    pub affected: u64,
}

/// Stable identity of an execution bound, suitable for a redacted limit receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    /// Emitted row count.
    Rows,
    /// Emitted cell count.
    Cells,
    /// Logical output bytes.
    Bytes,
    /// Provider work units.
    Work,
    /// Delivered-clock deadline.
    Deadline,
}

/// Receipt deliberately excludes locator, bindings, row values, and provider errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// Stable checked plan/program identity when applicable.
    pub operation_id: String,
    /// Public operation kind.
    pub operation: Operation,
    /// Rows pushed to the sink.
    pub rows: u64,
    /// Cells pushed to the sink.
    pub cells: u64,
    /// Logical bytes pushed to the sink.
    pub bytes: u64,
    /// Provider work units.
    pub work: u64,
    /// Rows affected.
    pub affected: u64,
}

/// Capability/effect kind selected at the operation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Read checked rows.
    Read,
    /// Apply a checked data mutation.
    Write,
    /// Apply an admitted schema operation.
    Schema,
    /// Apply an admitted migration.
    Migrate,
    /// Enter a closure-managed transaction.
    Transaction,
    /// Attach a provider-specific locator.
    Attach,
}
impl Operation {
    fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Schema => "schema",
            Self::Migrate => "migrate",
            Self::Transaction => "transaction",
            Self::Attach => "attach",
        }
    }
    pub(crate) fn capability(self) -> CapabilityName {
        CapabilityName::new(format!("relation.{}", self.label()))
    }
    pub(crate) fn effect(self) -> Symbol {
        Symbol::qualified("effect/relation", self.label())
    }
}

/// Site failure. Sensitive provider detail is never included in receipts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiteError {
    /// At least one mandatory limit was zero.
    InvalidLimits,
    /// Bindings violated the checked parameter contract.
    Bindings(String),
    /// Provider emitted a row with the wrong type.
    RowType,
    /// A mandatory or optional bound was exceeded.
    Limit(LimitKind),
    /// Provider rejected its opaque locator.
    Locator,
    /// A provider was undeclared or its manifest identity was malformed.
    Registration,
    /// A stable provider failure category.
    Constraint,
    /// The provider could not acquire its bounded lock.
    Locked,
    /// The requested operation exceeds storage authority.
    ReadOnly,
    /// Execution was interrupted by cancellation or deadline.
    Interrupted,
    /// Durable provider state is corrupt.
    Corruption,
    /// A provider value could not satisfy the admitted domain.
    Conversion,
    /// Live physical objects disagree with their attestation.
    Drift,
    /// Provider operation failed; detail is intentionally redacted.
    Provider,
    /// Kernel capability/effect machinery rejected the operation.
    Kernel(String),
}
impl fmt::Display for SiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SiteError {}

/// A provider session. It can receive only sealed checked operations.
pub trait Session {
    /// Execute a checked query and push typed rows.
    fn query(
        &mut self,
        plan: &CheckedQuery,
        bindings: &Bindings,
        limits: &Limits,
        sink: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError>;
    /// Execute a checked mutation and push typed returning rows.
    fn mutate(
        &mut self,
        plan: &CheckedMutation,
        bindings: &Bindings,
        limits: &Limits,
        sink: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError>;
    /// Apply an admitted migration program.
    fn migrate(
        &mut self,
        program: &CheckedProgram,
        limits: &Limits,
    ) -> Result<ProviderStats, SiteError>;
    /// Perform a provider schema operation using an admitted migration program.
    fn schema(
        &mut self,
        program: &CheckedProgram,
        limits: &Limits,
    ) -> Result<ProviderStats, SiteError>;
    /// Run a closure-managed transaction.
    fn transaction(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError>;
    /// Attach another provider-specific locator.
    fn attach(&mut self, locator: &Datum, limits: &Limits) -> Result<ProviderStats, SiteError>;
}

/// Transaction-scoped checked operations. Raw commit/rollback methods are absent.
pub trait Transaction: Session {
    /// Run a closure-managed savepoint; failure must roll back before returning.
    fn savepoint(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError>;
}

/// Provider factory. Locator validation occurs only inside `connect`.
pub trait Driver: Send + Sync {
    /// Validate the opaque locator and open a session.
    fn connect(&self, locator: &Datum, limits: &Limits) -> Result<Box<dyn Session>, SiteError>;
}
