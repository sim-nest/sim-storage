//! Checked relation-command specimens for discovery and conformance.
// conformance: closed relation command grammar and authorization boundary.

use super::*;
use sim_kernel::{Export, Lib};
use std::sync::{Arc, Mutex};

fn words(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_owned).collect()
}

#[derive(Default)]
struct Specimen {
    seen: Mutex<Vec<RelationCommand>>,
}
impl RelationCommands for Specimen {
    fn execute(&self, command: &RelationCommand) -> Result<String, CommandError> {
        self.seen.lock().unwrap().push(command.clone());
        Ok(match command {
            RelationCommand::Site {
                action: ReadAction::List,
                ..
            } => "provider\tsite\nSQLite\trelation/site/sqlite\n".into(),
            RelationCommand::Site { id: Some(id), .. } => {
                format!("logical={id}\nphysical=sqlite:v3\naccess=read-only\n")
            }
            RelationCommand::Schema {
                action: SchemaAction::Inspect,
                target,
                ..
            } => format!("logical={target}\nphysical=schema:legacy-v1\nmode=read-only\n"),
            RelationCommand::Schema {
                action: SchemaAction::Adopt,
                artifact: Some(id),
                authority: Some(auth),
                ..
            } if id == &auth.expected_plan => format!(
                "logical={id}\nphysical={id}\nplan={id}\nauthorized-by={}\nadopted\n",
                auth.product
            ),
            RelationCommand::Schema {
                action: SchemaAction::Adopt,
                artifact: Some(id),
                authority: Some(auth),
                ..
            } => {
                return Err(CommandError::new(format!(
                    "drift refusal: checked={id} physical={}",
                    auth.expected_plan
                )));
            }
            RelationCommand::Schema {
                artifact: Some(id),
                authority: Some(auth),
                ..
            }
            | RelationCommand::Migration {
                artifact: id,
                authority: Some(auth),
                ..
            } if id == &auth.expected_plan => format!(
                "logical={id}\nphysical=sqlite:v3\nplan={id}\nbounds=rows:100\nauthorized-by={}\napplied\n",
                auth.product
            ),
            RelationCommand::Migration {
                artifact,
                authority: None,
                ..
            } => {
                format!("logical=migration\nphysical=sqlite:v2\nplan={artifact}\nbounds=work:100\n")
            }
            RelationCommand::Query { plan, limit, .. } => {
                format!("Table\nplan={plan}\nrows<= {limit}\n")
            }
            RelationCommand::Mutation {
                plan,
                limit,
                authority,
                ..
            } if plan == &authority.expected_plan => format!(
                "plan={plan}\nbounds=rows:{limit}\nauthorized-by={}\naffected=1\n",
                authority.product
            ),
            RelationCommand::Mount { target } => {
                format!("Dir\nmount={target}\nlogical=/relation/customers\nprovider=SQLite\n")
            }
            _ => return Err(CommandError::new("checked plan identity mismatch")),
        })
    }
}

#[test]
fn fresh_database_and_provider_listing_specimen() {
    let c = parse(&words("relation site list")).unwrap();
    assert!(Specimen::default().execute(&c).unwrap().contains("SQLite"));
}
#[test]
fn old_file_inspection_is_read_only() {
    let c = parse(&words("relation schema inspect --site legacy")).unwrap();
    let out = Specimen::default().execute(&c).unwrap();
    assert!(out.contains("mode=read-only"));
}
#[test]
fn exact_adoption_renders_ids_authority_and_plan() {
    let c=parse(&words("relation schema adopt --site db --schema schema:v1 --authorize office --expect-plan schema:v1")).unwrap();
    let out = Specimen::default().execute(&c).unwrap();
    assert!(
        out.contains("logical=schema:v1")
            && out.contains("physical=schema:v1")
            && out.contains("authorized-by=office")
    );
}
#[test]
fn migration_drift_refuses_action() {
    let c=parse(&words("relation schema adopt --site db --schema schema:v2 --authorize office --expect-plan schema:v1")).unwrap();
    assert!(
        Specimen::default()
            .execute(&c)
            .unwrap_err()
            .to_string()
            .contains("drift refusal")
    );
}
#[test]
fn bounded_query_specimen() {
    let c = parse(&words(
        "relation query run --site db --plan query:customers --limit 5",
    ))
    .unwrap();
    assert!(
        Specimen::default()
            .execute(&c)
            .unwrap()
            .contains("rows<= 5")
    );
}
#[test]
fn authorized_mutation_specimen() {
    let c=parse(&words("relation mutation run --site db --plan mutation:add --limit 1 --authorize ledger --expect-plan mutation:add")).unwrap();
    assert!(
        Specimen::default()
            .execute(&c)
            .unwrap()
            .contains("affected=1")
    );
}
#[test]
fn mount_explanation_specimen() {
    let c = parse(&words("relation mount explain --mount customer-db")).unwrap();
    assert!(Specimen::default().execute(&c).unwrap().starts_with("Dir"));
}
#[test]
fn every_documented_operation_parses() {
    for input in [
        "relation site show --site db",
        "relation schema apply --site db --schema schema:v2 --authorize office --expect-plan schema:v2",
        "relation migration plan --site db --migration migration:v2",
        "relation migration apply --site db --migration migration:v2 --authorize office --expect-plan migration:v2",
    ] {
        parse(&words(input)).unwrap();
    }
}
#[test]
fn mutation_and_adoption_require_product_authority() {
    for input in [
        "relation mutation run --site db --plan p",
        "relation schema adopt --site db --schema s",
    ] {
        assert!(
            parse(&words(input))
                .unwrap_err()
                .to_string()
                .contains("--authorize")
        );
    }
}
#[test]
fn raw_sql_escape_is_absent() {
    assert!(
        parse(&words("relation query run --site db --sql SELECT"))
            .unwrap_err()
            .to_string()
            .contains("raw SQL")
    );
}
#[test]
fn library_exports_exact_relation_handoff() {
    let lib = RelationCommandLib::new(Arc::new(Specimen::default()));
    assert!(Lib::manifest(&lib).exports.iter().any(
        |e| matches!(e, Export::Function{symbol,..} if symbol==&relation_entrypoint_symbol())
    ));
}
