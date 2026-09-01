use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use sim_kernel::{
    Cx, Datum, DatumStore, Effect, Ref, Symbol,
    effect::{effect_abort_op_key, effect_resume_op_key, resolve_effect},
};
use sim_relation_core::{Row, RowType, ToRelationDatum};
use sim_relation_migrate::CheckedProgram;
use sim_relation_plan::{CheckedMutation, CheckedQuery};

use crate::{
    Bindings, Driver, Limits, Operation, ProviderStats, Receipt, RelationPlacement, RowSink,
    Session, SiteError, Transaction,
};

/// Provider-neutral bounded site.
pub struct RelationSite {
    pub(crate) placement: RelationPlacement,
    pub(crate) driver: Arc<dyn Driver>,
}
impl RelationSite {
    /// Binds a loaded placement to its driver.
    pub fn new(placement: RelationPlacement, driver: Arc<dyn Driver>) -> Self {
        Self { placement, driver }
    }

    /// Execute a checked query.
    pub fn query(
        &self,
        cx: &mut Cx,
        plan: &CheckedQuery,
        bindings: &Bindings,
        limits: Limits,
        sink: &mut dyn RowSink,
    ) -> Result<Receipt, SiteError> {
        self.rows_effect(
            cx,
            Operation::Read,
            RowsSpec {
                id: content_id_text(plan.plan_id().content_id()),
                expected: plan.output(),
            },
            limits,
            sink,
            |session, bounded| session.query(plan, bindings, &limits, bounded),
        )
    }
    /// Execute a checked mutation.
    pub fn mutate(
        &self,
        cx: &mut Cx,
        plan: &CheckedMutation,
        bindings: &Bindings,
        limits: Limits,
        sink: &mut dyn RowSink,
    ) -> Result<Receipt, SiteError> {
        self.rows_effect(
            cx,
            Operation::Write,
            RowsSpec {
                id: content_id_text(plan.plan_id().content_id()),
                expected: plan.output(),
            },
            limits,
            sink,
            |session, bounded| session.mutate(plan, bindings, &limits, bounded),
        )
    }
    /// Apply a checked migration.
    pub fn migrate(
        &self,
        cx: &mut Cx,
        program: &CheckedProgram,
        limits: Limits,
    ) -> Result<Receipt, SiteError> {
        self.simple_effect(
            cx,
            Operation::Migrate,
            "checked-migration".into(),
            limits,
            |s| s.migrate(program, &limits),
        )
    }
    /// Apply an admitted schema program under the schema capability.
    pub fn schema(
        &self,
        cx: &mut Cx,
        program: &CheckedProgram,
        limits: Limits,
    ) -> Result<Receipt, SiteError> {
        self.simple_effect(
            cx,
            Operation::Schema,
            "checked-schema".into(),
            limits,
            |s| s.schema(program, &limits),
        )
    }
    /// Attach a provider locator under the attach capability.
    pub fn attach(
        &self,
        cx: &mut Cx,
        locator: &Datum,
        limits: Limits,
    ) -> Result<Receipt, SiteError> {
        self.simple_effect(cx, Operation::Attach, "attach".into(), limits, |s| {
            s.attach(locator, &limits)
        })
    }
    /// Run a closure-managed transaction. Provider unwind must roll back.
    pub fn transaction(
        &self,
        cx: &mut Cx,
        limits: Limits,
        mut body: impl FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<Receipt, SiteError> {
        self.simple_effect(
            cx,
            Operation::Transaction,
            "transaction".into(),
            limits,
            |s| {
                s.transaction(&mut body)?;
                Ok(ProviderStats::default())
            },
        )
    }

    fn connect(&self, limits: &Limits) -> Result<Box<dyn Session>, SiteError> {
        self.driver
            .connect(self.placement.locator(), limits)
            .map_err(|_| SiteError::Locator)
    }
    fn simple_effect(
        &self,
        cx: &mut Cx,
        op: Operation,
        id: String,
        limits: Limits,
        perform: impl FnOnce(&mut dyn Session) -> Result<ProviderStats, SiteError>,
    ) -> Result<Receipt, SiteError> {
        let mut stats = None;
        self.effect(cx, op, |cx| {
            let mut session = self.connect(&limits)?;
            let started = Instant::now();
            let got = perform(session.as_mut())?;
            enforce_deadline(&limits, started.elapsed())?;
            enforce_work(&limits, got.work)?;
            stats = Some(got);
            cx.datum_store_mut()
                .intern(Datum::Symbol(Symbol::new("ok")))
                .map(Ref::Content)
                .map_err(kernel)
        })?;
        let got = stats.unwrap_or_default();
        Ok(Receipt {
            operation_id: id,
            operation: op,
            rows: 0,
            cells: 0,
            bytes: 0,
            work: got.work,
            affected: got.affected,
        })
    }
    fn rows_effect(
        &self,
        cx: &mut Cx,
        op: Operation,
        spec: RowsSpec<'_>,
        limits: Limits,
        sink: &mut dyn RowSink,
        perform: impl FnOnce(&mut dyn Session, &mut dyn RowSink) -> Result<ProviderStats, SiteError>,
    ) -> Result<Receipt, SiteError> {
        let mut counts = Counts::default();
        let mut stats = None;
        self.effect(cx, op, |cx| {
            let mut session = self.connect(&limits)?;
            let mut bounded = BoundedSink {
                expected: spec.expected,
                limits: &limits,
                inner: sink,
                counts: &mut counts,
            };
            let started = Instant::now();
            let got = perform(session.as_mut(), &mut bounded)?;
            enforce_deadline(&limits, started.elapsed())?;
            enforce_work(&limits, got.work)?;
            stats = Some(got);
            cx.datum_store_mut()
                .intern(Datum::Symbol(Symbol::new("ok")))
                .map(Ref::Content)
                .map_err(kernel)
        })?;
        let got = stats.unwrap_or_default();
        Ok(Receipt {
            operation_id: spec.id,
            operation: op,
            rows: counts.rows,
            cells: counts.cells,
            bytes: counts.bytes,
            work: got.work,
            affected: got.affected,
        })
    }
    pub(crate) fn effect(
        &self,
        cx: &mut Cx,
        op: Operation,
        perform: impl FnOnce(&mut Cx) -> Result<Ref, SiteError>,
    ) -> Result<(), SiteError> {
        let input = cx
            .datum_store_mut()
            .intern(Datum::Node {
                tag: Symbol::qualified("relation", "request"),
                fields: vec![(
                    Symbol::new("site"),
                    Datum::Symbol(self.placement.site.clone()),
                )],
            })
            .map(Ref::Content)
            .map_err(kernel)?;
        let effect = Effect::new(
            cx.fresh_handle(),
            op.effect(),
            Ref::Symbol(self.placement.site.clone()),
            input,
            Ref::Symbol(Symbol::qualified("core", "Any")),
            effect_resume_op_key(),
            effect_abort_op_key(),
        )
        .requiring(op.capability());
        let mut operation_error = None;
        if let Err(error) = resolve_effect(cx, effect, |cx, _| match perform(cx) {
            Ok(value) => Ok(value),
            Err(error) => {
                operation_error = Some(error.clone());
                Err(sim_kernel::Error::Eval(error.to_string()))
            }
        }) {
            return Err(operation_error.unwrap_or_else(|| kernel(error)));
        }
        Ok(())
    }
}

struct RowsSpec<'a> {
    id: String,
    expected: &'a RowType,
}

#[derive(Default)]
pub(crate) struct Counts {
    rows: u64,
    cells: u64,
    bytes: u64,
}
pub(crate) struct BoundedSink<'a> {
    pub(crate) expected: &'a RowType,
    pub(crate) limits: &'a Limits,
    pub(crate) inner: &'a mut dyn RowSink,
    pub(crate) counts: &'a mut Counts,
}
impl RowSink for BoundedSink<'_> {
    fn push(&mut self, row: Row) -> Result<(), SiteError> {
        if row.row_type() != self.expected {
            return Err(SiteError::RowType);
        }
        let cells = row.cells().len() as u64;
        let bytes = format!("{:?}", row.to_datum()).len() as u64;
        let rows = self
            .counts
            .rows
            .checked_add(1)
            .ok_or(SiteError::Limit(crate::LimitKind::Rows))?;
        let cells = self
            .counts
            .cells
            .checked_add(cells)
            .ok_or(SiteError::Limit(crate::LimitKind::Cells))?;
        let bytes = self
            .counts
            .bytes
            .checked_add(bytes)
            .ok_or(SiteError::Limit(crate::LimitKind::Bytes))?;
        if rows > self.limits.rows {
            return Err(SiteError::Limit(crate::LimitKind::Rows));
        }
        if cells > self.limits.cells {
            return Err(SiteError::Limit(crate::LimitKind::Cells));
        }
        if bytes > self.limits.bytes {
            return Err(SiteError::Limit(crate::LimitKind::Bytes));
        }
        self.inner.push(row)?;
        self.counts.rows = rows;
        self.counts.cells = cells;
        self.counts.bytes = bytes;
        Ok(())
    }
}
pub(crate) fn enforce_work(l: &Limits, work: u64) -> Result<(), SiteError> {
    if work > l.work {
        Err(SiteError::Limit(crate::LimitKind::Work))
    } else {
        Ok(())
    }
}
fn enforce_deadline(limits: &Limits, elapsed: Duration) -> Result<(), SiteError> {
    if limits.deadline.is_some_and(|deadline| elapsed > deadline) {
        Err(SiteError::Limit(crate::LimitKind::Deadline))
    } else {
        Ok(())
    }
}
fn content_id_text(id: &sim_kernel::ContentId) -> String {
    let digest: String = id.bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{}:{digest}", id.algorithm)
}
pub(crate) fn kernel(e: sim_kernel::Error) -> SiteError {
    SiteError::Kernel(e.to_string())
}
