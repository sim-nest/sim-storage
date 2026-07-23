//! Shared routing helpers for mounted namespaces.

use std::collections::BTreeMap;

use sim_kernel::{
    Cx, Error, Expr, Result, Symbol, Value,
    table::{Dir, Table},
};
use sim_table_core::{TablePath, is_legal_table_segment};

use crate::{
    MountInspection, MountKind,
    mount_dir::{MountPath, MountTarget},
};

pub(crate) enum ResolvedNode {
    Backed(Value),
    Virtual,
}

pub(crate) fn resolve_mounted_target(
    cx: &mut Cx,
    target: MountTarget,
    suffix: &[String],
) -> Result<ResolvedNode> {
    match target.kind {
        MountKind::Table if suffix.is_empty() => Ok(ResolvedNode::Backed(target.value)),
        MountKind::Table => Err(Error::Eval(
            "table/mount: cannot traverse below a table mount".to_owned(),
        )),
        MountKind::Dir => traverse_dir_path(cx, target.value, suffix)?
            .map(ResolvedNode::Backed)
            .ok_or_else(|| {
                Error::Eval("table/mount: mounted directory path is missing".to_owned())
            }),
    }
}

pub(crate) fn traverse_dir_path(
    cx: &mut Cx,
    root: Value,
    path: &[String],
) -> Result<Option<Value>> {
    let mut current = root;
    for segment in path {
        let dir = require_dir_value(&current, "table/mount: path component is not a Dir")?;
        let Some(next) = dir.opendir(cx, Symbol::new(segment.as_str()))? else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

pub(crate) fn longest_mount(
    mounts: &BTreeMap<MountPath, MountTarget>,
    path: &[String],
) -> Option<(usize, MountTarget)> {
    mounts
        .iter()
        .filter(|(mount_path, _)| is_prefix(mount_path, path))
        .max_by_key(|(mount_path, _)| mount_path.len())
        .map(|(mount_path, target)| (mount_path.len(), target.clone()))
}

pub(crate) fn has_mount_descendant(
    mounts: &BTreeMap<MountPath, MountTarget>,
    path: &[String],
) -> bool {
    mounts
        .keys()
        .any(|mount_path| mount_path.len() > path.len() && is_prefix(path, mount_path))
}

pub(crate) fn require_table_value<'a>(value: &'a Value, message: &str) -> Result<&'a dyn Table> {
    value
        .object()
        .as_table_impl()
        .ok_or_else(|| Error::Eval(message.to_owned()))
}

pub(crate) fn require_dir_value<'a>(value: &'a Value, message: &str) -> Result<&'a dyn Dir> {
    value
        .object()
        .as_dir()
        .ok_or_else(|| Error::Eval(message.to_owned()))
}

pub(crate) fn check_segment(name: &Symbol) -> Result<()> {
    if is_legal_table_segment(&name.name) {
        Ok(())
    } else {
        Err(Error::Eval(format!(
            "table/mount: illegal path segment {:?}",
            name.name
        )))
    }
}

pub(crate) fn owned_path(path: &TablePath) -> MountPath {
    path.segments().to_vec()
}

pub(crate) fn is_prefix(prefix: &[String], path: &[String]) -> bool {
    path.len() >= prefix.len() && path.iter().zip(prefix).all(|(left, right)| left == right)
}

pub(crate) fn format_path(path: &[String]) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", path.join("/"))
    }
}

pub(crate) fn join_child(path: &[String], name: &Symbol) -> String {
    let mut child = path.to_vec();
    child.push(name.name.to_string());
    format_path(&child)
}

pub(crate) fn inspection_expr(rows: &[MountInspection]) -> Expr {
    Expr::List(
        rows.iter()
            .map(|row| {
                Expr::Map(vec![
                    (
                        Expr::Symbol(Symbol::new("path")),
                        Expr::String(row.path.clone()),
                    ),
                    (
                        Expr::Symbol(Symbol::new("kind")),
                        Expr::Symbol(Symbol::qualified("table/mount", row.kind.label())),
                    ),
                ])
            })
            .collect(),
    )
}
