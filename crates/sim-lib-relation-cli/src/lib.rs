#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Loadable command surface for checked relational data.

use std::{fmt, sync::Arc};

use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Expr, Lib, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result as KernelResult, Symbol, Value, Version,
};

/// Stable default maximum number of rows returned by a command.
pub const DEFAULT_ROW_LIMIT: u64 = 100;

/// A parsed, provider-neutral relation operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationCommand {
    /// List loaded sites or show one site's public contract.
    Site {
        /// Requested read operation.
        action: ReadAction,
        /// Site identity for `show`.
        id: Option<String>,
    },
    /// Inspect, adopt, or apply a named checked schema.
    Schema {
        /// Requested schema operation.
        action: SchemaAction,
        /// Loaded logical site identity.
        target: String,
        /// Named checked schema, when required.
        artifact: Option<String>,
        /// Product authority, when the operation can alter state.
        authority: Option<Authority>,
    },
    /// Plan or apply a named checked migration.
    Migration {
        /// Plan or apply.
        action: ApplyAction,
        /// Loaded logical site identity.
        target: String,
        /// Named checked migration.
        artifact: String,
        /// Product authority for apply.
        authority: Option<Authority>,
    },
    /// Run a named checked query with a mandatory row bound.
    Query {
        /// Loaded logical site identity.
        target: String,
        /// Named checked query plan.
        plan: String,
        /// Maximum returned rows.
        limit: u64,
    },
    /// Run a named checked mutation with explicit product authority.
    Mutation {
        /// Loaded logical site identity.
        target: String,
        /// Named checked mutation plan.
        plan: String,
        /// Maximum returned rows.
        limit: u64,
        /// Product authority bound to the plan.
        authority: Authority,
    },
    /// Explain a named logical mount without exposing provider paths.
    Mount {
        /// Logical mount identity.
        target: String,
    },
}

/// List/show choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadAction {
    /// List public records.
    List,
    /// Show one public record.
    Show,
}
/// Schema operation choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchemaAction {
    /// Inspect physical state.
    Inspect,
    /// Adopt an exact physical schema.
    Adopt,
    /// Apply an admitted schema.
    Apply,
}
/// Plan/apply choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyAction {
    /// Render the checked plan.
    Plan,
    /// Apply the checked plan.
    Apply,
}

/// Explicit product authorization bound to the checked logical/physical plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authority {
    /// Product identity granting the operation.
    pub product: String,
    /// Exact checked plan identity the product reviewed.
    pub expected_plan: String,
}

/// Stable command error without provider-sensitive detail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError(String);
impl CommandError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
    /// Constructs a redaction-safe availability failure for platform adapters.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CommandError {}

/// Provider-facing command boundary. Implementations receive names and bounds,
/// never SQL text or native paths.
pub trait RelationCommands: Send + Sync {
    /// Executes one admitted operation and returns Table/Dir-oriented text.
    fn execute(&self, command: &RelationCommand) -> Result<String, CommandError>;
}

/// Parses the closed `sim relation` grammar.
pub fn parse(args: &[String]) -> Result<RelationCommand, CommandError> {
    let args = if args.first().is_some_and(|v| v == "relation") {
        &args[1..]
    } else {
        args
    };
    if args.iter().any(|v| v == "--sql" || v.starts_with("--sql=")) {
        return Err(CommandError::new(
            "raw SQL is not an admitted relation command",
        ));
    }
    let (family, action) = match args {
        [family, action, ..] => (family.as_str(), action.as_str()),
        _ => return Err(usage()),
    };
    let opts = Options::parse(&args[2..])?;
    match (family, action) {
        ("site", "list") => {
            opts.reject_all()?;
            Ok(RelationCommand::Site {
                action: ReadAction::List,
                id: None,
            })
        }
        ("site", "show") => Ok(RelationCommand::Site {
            action: ReadAction::Show,
            id: Some(opts.required("--site")?),
        }),
        ("schema", "inspect") => Ok(RelationCommand::Schema {
            action: SchemaAction::Inspect,
            target: opts.required("--site")?,
            artifact: None,
            authority: None,
        }),
        ("schema", "adopt") => Ok(RelationCommand::Schema {
            action: SchemaAction::Adopt,
            target: opts.required("--site")?,
            artifact: Some(opts.required("--schema")?),
            authority: Some(opts.authority()?),
        }),
        ("schema", "apply") => Ok(RelationCommand::Schema {
            action: SchemaAction::Apply,
            target: opts.required("--site")?,
            artifact: Some(opts.required("--schema")?),
            authority: Some(opts.authority()?),
        }),
        ("migration", "plan") => Ok(RelationCommand::Migration {
            action: ApplyAction::Plan,
            target: opts.required("--site")?,
            artifact: opts.required("--migration")?,
            authority: None,
        }),
        ("migration", "apply") => Ok(RelationCommand::Migration {
            action: ApplyAction::Apply,
            target: opts.required("--site")?,
            artifact: opts.required("--migration")?,
            authority: Some(opts.authority()?),
        }),
        ("query", "run") => Ok(RelationCommand::Query {
            target: opts.required("--site")?,
            plan: opts.required("--plan")?,
            limit: opts.limit()?,
        }),
        ("mutation", "run") => Ok(RelationCommand::Mutation {
            target: opts.required("--site")?,
            plan: opts.required("--plan")?,
            limit: opts.limit()?,
            authority: opts.authority()?,
        }),
        ("mount", "explain") => Ok(RelationCommand::Mount {
            target: opts.required("--mount")?,
        }),
        _ => Err(usage()),
    }
}

fn usage() -> CommandError {
    CommandError::new(
        "usage: sim relation {site list|show|schema inspect|adopt|apply|migration plan|apply|query run|mutation run|mount explain}",
    )
}

#[derive(Default)]
struct Options(Vec<(String, String)>);
impl Options {
    fn parse(args: &[String]) -> Result<Self, CommandError> {
        if !args.len().is_multiple_of(2) {
            return Err(CommandError::new(
                "every relation option requires one value",
            ));
        }
        let mut out = Vec::new();
        for pair in args.chunks_exact(2) {
            if !pair[0].starts_with("--") {
                return Err(CommandError::new(format!(
                    "unexpected argument: {}",
                    pair[0]
                )));
            }
            if out.iter().any(|(key, _)| key == &pair[0]) {
                return Err(CommandError::new(format!("duplicate option: {}", pair[0])));
            }
            out.push((pair[0].clone(), pair[1].clone()));
        }
        Ok(Self(out))
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
    fn required(&self, key: &str) -> Result<String, CommandError> {
        self.get(key)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| CommandError::new(format!("missing {key}")))
    }
    fn limit(&self) -> Result<u64, CommandError> {
        match self.get("--limit") {
            None => Ok(DEFAULT_ROW_LIMIT),
            Some(v) => v
                .parse()
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| CommandError::new("--limit must be a positive integer")),
        }
    }
    fn authority(&self) -> Result<Authority, CommandError> {
        Ok(Authority {
            product: self.required("--authorize")?,
            expected_plan: self.required("--expect-plan")?,
        })
    }
    fn reject_all(&self) -> Result<(), CommandError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(CommandError::new("site list accepts no options"))
        }
    }
}

/// Loadable relation command library.
#[derive(Clone)]
pub struct RelationCommandLib {
    backend: Arc<dyn RelationCommands>,
}
impl RelationCommandLib {
    /// Creates a command library over a provider-composed backend.
    pub fn new(backend: Arc<dyn RelationCommands>) -> Self {
        Self { backend }
    }
}

impl Lib for RelationCommandLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "relation-cli"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: relation_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }
    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> KernelResult<()> {
        linker.function_value(
            relation_entrypoint_symbol(),
            cx.factory()
                .opaque(Arc::new(Entrypoint(self.backend.clone())))?,
        )?;
        Ok(())
    }
}

/// Exact bootloader handoff symbol.
pub fn relation_entrypoint_symbol() -> Symbol {
    Symbol::qualified("cli", "main/relation")
}

#[derive(Clone)]
struct Entrypoint(Arc<dyn RelationCommands>);
impl Object for Entrypoint {
    fn display(&self, _: &mut Cx) -> KernelResult<String> {
        Ok("cli/main/relation".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for Entrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for Entrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> KernelResult<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing relation envelope".into()))?;
        let argv = envelope_args(cx, envelope)?;
        let command = parse(&argv).map_err(|e| Error::Eval(e.to_string()))?;
        let output = self
            .0
            .execute(&command)
            .map_err(|e| Error::Eval(e.to_string()))?;
        print!("{output}");
        cx.factory().bool(true)
    }
}
fn envelope_args(cx: &mut Cx, envelope: &Value) -> KernelResult<Vec<String>> {
    let table = envelope
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval("relation CLI envelope is not a table".into()))?;
    let value = table.get(cx, Symbol::new("args"))?;
    let Expr::List(items) = value.object().as_expr(cx)? else {
        return Err(Error::TypeMismatch {
            expected: "argument list",
            found: "non-list",
        });
    };
    items
        .into_iter()
        .map(|item| {
            if let Expr::String(v) = item {
                Ok(v)
            } else {
                Err(Error::TypeMismatch {
                    expected: "string argument",
                    found: "non-string",
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
