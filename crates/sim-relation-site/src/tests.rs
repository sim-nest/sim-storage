//! Relation-site conformance: the recording driver proves the bounded host seam.

use super::*;
use sim_kernel::{CapabilityName, Datum, DatumStore, Ref, testing::bare_cx as cx};
use sim_relation_core::{Cell, DomainId, FieldName, FieldType, Row, RowType};
use sim_relation_migrate::CheckedProgram;
use sim_relation_plan::{CheckedMutation, CheckedQuery};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Log(Mutex<Vec<&'static str>>);
struct RecordingDriver {
    log: Arc<Log>,
    fail: bool,
}
struct RecordingSession {
    log: Arc<Log>,
}
impl Driver for RecordingDriver {
    fn connect(&self, locator: &Datum, _: &Limits) -> Result<Box<dyn Session>, SiteError> {
        self.log.0.lock().unwrap().push("connect");
        if self.fail || !matches!(locator, Datum::String(_)) {
            Err(SiteError::Locator)
        } else {
            Ok(Box::new(RecordingSession {
                log: self.log.clone(),
            }))
        }
    }
}
impl Session for RecordingSession {
    fn query(
        &mut self,
        _: &CheckedQuery,
        _: &Bindings,
        _: &Limits,
        _: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        self.log.0.lock().unwrap().push("query");
        Ok(ProviderStats {
            work: 1,
            affected: 0,
        })
    }
    fn mutate(
        &mut self,
        _: &CheckedMutation,
        _: &Bindings,
        _: &Limits,
        _: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        self.log.0.lock().unwrap().push("mutate");
        Ok(ProviderStats {
            work: 1,
            affected: 1,
        })
    }
    fn migrate(&mut self, _: &CheckedProgram, _: &Limits) -> Result<ProviderStats, SiteError> {
        self.log.0.lock().unwrap().push("migrate");
        Ok(ProviderStats {
            work: 1,
            affected: 0,
        })
    }
    fn schema(&mut self, _: &CheckedProgram, _: &Limits) -> Result<ProviderStats, SiteError> {
        self.log.0.lock().unwrap().push("schema");
        Ok(ProviderStats {
            work: 1,
            affected: 0,
        })
    }
    fn transaction(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError> {
        self.log.0.lock().unwrap().push("begin");
        let mut tx = RecordingTx {
            log: self.log.clone(),
        };
        match body(&mut tx) {
            Ok(()) => {
                self.log.0.lock().unwrap().push("commit");
                Ok(())
            }
            Err(e) => {
                self.log.0.lock().unwrap().push("rollback");
                Err(e)
            }
        }
    }
    fn attach(&mut self, _: &Datum, _: &Limits) -> Result<ProviderStats, SiteError> {
        self.log.0.lock().unwrap().push("attach");
        Ok(ProviderStats::default())
    }
}
struct RecordingTx {
    log: Arc<Log>,
}
impl Session for RecordingTx {
    fn query(
        &mut self,
        _: &CheckedQuery,
        _: &Bindings,
        _: &Limits,
        _: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        Ok(ProviderStats::default())
    }
    fn mutate(
        &mut self,
        _: &CheckedMutation,
        _: &Bindings,
        _: &Limits,
        _: &mut dyn RowSink,
    ) -> Result<ProviderStats, SiteError> {
        Ok(ProviderStats::default())
    }
    fn migrate(&mut self, _: &CheckedProgram, _: &Limits) -> Result<ProviderStats, SiteError> {
        Ok(ProviderStats::default())
    }
    fn schema(&mut self, _: &CheckedProgram, _: &Limits) -> Result<ProviderStats, SiteError> {
        Ok(ProviderStats::default())
    }
    fn transaction(
        &mut self,
        _: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError> {
        Err(SiteError::Provider)
    }
    fn attach(&mut self, _: &Datum, _: &Limits) -> Result<ProviderStats, SiteError> {
        Ok(ProviderStats::default())
    }
}
impl Transaction for RecordingTx {
    fn savepoint(
        &mut self,
        body: &mut dyn FnMut(&mut dyn Transaction) -> Result<(), SiteError>,
    ) -> Result<(), SiteError> {
        self.log.0.lock().unwrap().push("savepoint");
        match body(self) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.log.0.lock().unwrap().push("rollback-savepoint");
                Err(e)
            }
        }
    }
}
fn limits() -> Limits {
    Limits::new(2, 2, 1000, 2).unwrap()
}
fn site(log: Arc<Log>) -> RelationSite {
    RelationSite::new(
        RelationPlacement::new(
            Symbol::new("site/relation/recording"),
            Datum::String("opaque".into()),
        ),
        Arc::new(RecordingDriver { log, fail: false }),
    )
}

#[test]
fn capability_denial_precedes_provider_contact() {
    let log = Arc::new(Log::default());
    let mut cx = cx();
    let err = site(log.clone())
        .attach(&mut cx, &Datum::Nil, limits())
        .unwrap_err();
    assert!(matches!(err, SiteError::Kernel(_)));
    assert_eq!(cx.effect_ledger().records().len(), 1);
    assert!(cx.effect_ledger().records()[0].aborted);
    assert!(log.0.lock().unwrap().is_empty());
}
#[test]
fn transaction_and_savepoint_unwind_totally() {
    let log = Arc::new(Log::default());
    let mut cx = cx();
    cx.grant(CapabilityName::new("relation.transaction"));
    let err = site(log.clone())
        .transaction(&mut cx, limits(), |tx| {
            tx.savepoint(&mut |_tx| Err(SiteError::Provider))
        })
        .unwrap_err();
    assert!(matches!(err, SiteError::Provider));
    assert_eq!(cx.effect_ledger().records().len(), 1);
    assert!(cx.effect_ledger().records()[0].aborted);
    assert_eq!(
        *log.0.lock().unwrap(),
        vec![
            "connect",
            "begin",
            "savepoint",
            "rollback-savepoint",
            "rollback"
        ]
    );
}
#[test]
fn locator_is_validated_only_after_capability() {
    let log = Arc::new(Log::default());
    let mut cx = cx();
    cx.grant(CapabilityName::new("relation.attach"));
    let placement = RelationPlacement::new(Symbol::new("site/relation/recording"), Datum::Nil);
    let s = RelationSite::new(
        placement,
        Arc::new(RecordingDriver {
            log: log.clone(),
            fail: false,
        }),
    );
    assert!(matches!(
        s.attach(&mut cx, &Datum::Nil, limits()),
        Err(SiteError::Locator)
    ));
    assert_eq!(*log.0.lock().unwrap(), vec!["connect"]);
}
#[test]
fn mandatory_limits_and_bindings_fail_closed() {
    assert!(matches!(
        Limits::new(0, 1, 1, 1),
        Err(SiteError::InvalidLimits)
    ));
    let domain = DomainId::new(Symbol::qualified("domain", "text")).unwrap();
    let row_type = RowType::new([FieldType {
        name: FieldName::new(Symbol::new("value")).unwrap(),
        domain: domain.clone(),
        nullable: false,
    }])
    .unwrap();
    assert!(matches!(
        Bindings::new(&row_type, []),
        Err(SiteError::Bindings(_))
    ));

    let row = Row::new(
        row_type.clone(),
        [Cell::new(domain, Some(Datum::String("bounded".into())))],
    )
    .unwrap();
    let mut rows = Vec::new();
    struct Collect<'a>(&'a mut Vec<Row>);
    impl RowSink for Collect<'_> {
        fn push(&mut self, row: Row) -> Result<(), SiteError> {
            self.0.push(row);
            Ok(())
        }
    }
    let limits = Limits::new(1, 1, 1_000, 1).unwrap();
    let mut counts = Counts::default();
    let mut collect = Collect(&mut rows);
    let mut sink = BoundedSink {
        expected: &row_type,
        limits: &limits,
        inner: &mut collect,
        counts: &mut counts,
    };
    sink.push(row.clone()).unwrap();
    assert!(matches!(
        sink.push(row),
        Err(SiteError::Limit(LimitKind::Rows))
    ));
    assert_eq!(rows.len(), 1);
    assert!(matches!(
        enforce_work(&limits, 2),
        Err(SiteError::Limit(LimitKind::Work))
    ));
}

#[test]
fn sqlite_registration_and_locator_grammar_fail_closed() {
    let manifest = DriverManifest::sqlite(
        Symbol::qualified("relation/site", "sqlite"),
        Symbol::qualified("relation/provider", "sqlite"),
    )
    .unwrap();
    assert_eq!(manifest.site, Symbol::qualified("relation/site", "sqlite"));
    assert!(matches!(
        DriverManifest::sqlite(Symbol::new("sqlite"), Symbol::new("sqlite")),
        Err(SiteError::Registration)
    ));
    assert_eq!(
        StorageLocator::from_datum(&Datum::Node {
            tag: Symbol::qualified("relation", "memory"),
            fields: vec![],
        }),
        Ok(StorageLocator::Memory)
    );
    assert!(matches!(
        StorageLocator::from_datum(&Datum::String("/tmp/db".into())),
        Err(SiteError::Locator)
    ));
}

#[test]
fn every_operation_is_capability_gated_and_records_one_effect() {
    for operation in [
        Operation::Read,
        Operation::Write,
        Operation::Schema,
        Operation::Migrate,
        Operation::Transaction,
        Operation::Attach,
    ] {
        let log = Arc::new(Log::default());
        let relation_site = site(log.clone());
        let mut denied = cx();
        let error = relation_site
            .effect(&mut denied, operation, |_| panic!("denied operation ran"))
            .unwrap_err();
        assert!(matches!(error, SiteError::Kernel(_)));
        assert_eq!(denied.effect_ledger().records().len(), 1);
        assert!(denied.effect_ledger().records()[0].aborted);

        let mut allowed = cx();
        allowed.grant(operation.capability());
        relation_site
            .effect(&mut allowed, operation, |cx| {
                cx.datum_store_mut()
                    .intern(Datum::Nil)
                    .map(Ref::Content)
                    .map_err(kernel)
            })
            .unwrap();
        assert_eq!(allowed.effect_ledger().records().len(), 1);
        assert!(!allowed.effect_ledger().records()[0].aborted);
    }
}
#[test]
fn library_declares_site_export() {
    let lib = RelationSiteLib::new(site(Arc::new(Log::default())));
    assert!(
        matches!(&lib.manifest().exports[0],Export::Site{symbol,..} if symbol==&Symbol::new("site/relation/recording"))
    );
}
