// conformance: SQLite registration and locators remain closed and capability-bearing.

use sim_kernel::{Datum, Symbol};

use crate::{DriverManifest, SiteError, StorageLocator};

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
