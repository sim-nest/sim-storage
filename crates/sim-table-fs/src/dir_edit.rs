//! Atomic text edits for filesystem-backed table leaves.

use std::{
    fs::OpenOptions,
    io::{ErrorKind, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use sim_kernel::{Cx, Error, Expr, Result, Symbol};

use crate::{
    FsDir,
    capabilities::{require_table_fs_edit, require_table_fs_read, require_table_fs_write},
};

impl FsDir {
    /// Applies an exact text replacement to a string leaf and atomically writes it.
    ///
    /// Requires `fs/read`, `fs/write`, and `edit`.
    pub fn edit(
        &self,
        cx: &mut Cx,
        key: Symbol,
        old: &str,
        new: &str,
        replace_all: bool,
    ) -> Result<()> {
        self.edit_text_leaf(cx, key, |text| apply_edit(text, old, new, replace_all))
    }

    /// Applies a 1-based inclusive line-range replacement to a string leaf.
    ///
    /// Requires `fs/read`, `fs/write`, and `edit`.
    pub fn edit_lines(
        &self,
        cx: &mut Cx,
        key: Symbol,
        start: usize,
        end: usize,
        new: &str,
    ) -> Result<()> {
        self.edit_text_leaf(cx, key, |text| apply_edit_lines(text, start, end, new))
    }

    fn edit_text_leaf<F>(&self, cx: &mut Cx, key: Symbol, edit: F) -> Result<()>
    where
        F: FnOnce(&str) -> Result<String>,
    {
        require_table_fs_read(cx)?;
        require_table_fs_write(cx)?;
        require_table_fs_edit(cx)?;

        let (path, ext, expr) = self.read_leaf_expr(cx, &key)?;
        let Expr::String(text) = expr else {
            return Err(Error::Eval(format!(
                "table/fs: dir/edit expects string leaf at {key}"
            )));
        };
        let edited = edit(&text)?;
        let bytes = FsDir::encode_leaf_expr(cx, ext, &Expr::String(edited))?;
        self.atomic_write_leaf(&path, &bytes)
    }

    fn atomic_write_leaf(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.ensure_internal_path(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| Error::Eval("table/fs: edit target has no parent".to_owned()))?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Eval("table/fs: edit target has invalid name".to_owned()))?;
        for attempt in 0..32 {
            let temp_name = format!(
                ".{file_name}.edit-{}-{}-{attempt}.tmp",
                std::process::id(),
                unique_nanos()
            );
            let temp_path = parent.join(temp_name);
            self.ensure_internal_path(&temp_path)?;
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
            {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(bytes) {
                        let _ = std::fs::remove_file(&temp_path);
                        return Err(Error::Eval(format!("table/fs: edit write {err}")));
                    }
                    if let Err(err) = file.sync_all() {
                        let _ = std::fs::remove_file(&temp_path);
                        return Err(Error::Eval(format!("table/fs: edit sync {err}")));
                    }
                    drop(file);
                    if let Err(err) = std::fs::rename(&temp_path, path) {
                        let _ = std::fs::remove_file(&temp_path);
                        return Err(Error::Eval(format!("table/fs: edit rename {err}")));
                    }
                    return Ok(());
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(Error::Eval(format!("table/fs: edit temp {err}"))),
            }
        }
        Err(Error::Eval(
            "table/fs: edit could not create a temporary file".to_owned(),
        ))
    }
}

fn unique_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn apply_edit(text: &str, old: &str, new: &str, replace_all: bool) -> Result<String> {
    if old.is_empty() {
        return Err(Error::Eval("edit: old pattern is empty".to_owned()));
    }
    let matches = text.matches(old).count();
    match matches {
        0 => Err(Error::Eval(format!("edit: pattern not found: {old:?}"))),
        n if n > 1 && !replace_all => Err(Error::Eval(format!(
            "edit: pattern is not unique ({n} matches); pass replace_all"
        ))),
        _ if replace_all => Ok(text.replace(old, new)),
        _ => Ok(text.replacen(old, new, 1)),
    }
}

fn apply_edit_lines(text: &str, start: usize, end: usize, new: &str) -> Result<String> {
    if start == 0 {
        return Err(Error::Eval(
            "edit-lines: start must be at least 1".to_owned(),
        ));
    }
    if end < start {
        return Err(Error::Eval(
            "edit-lines: end must be greater than or equal to start".to_owned(),
        ));
    }
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if end > lines.len() {
        return Err(Error::Eval(format!(
            "edit-lines: range {start}..{end} exceeds {} line(s)",
            lines.len()
        )));
    }
    let mut edited = String::new();
    for line in &lines[..start - 1] {
        edited.push_str(line);
    }
    edited.push_str(new);
    for line in &lines[end..] {
        edited.push_str(line);
    }
    Ok(edited)
}
