//! Read-only grep and glob search for filesystem-backed tables.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;
use sim_kernel::{Cx, Error, Expr, Result};

use crate::{
    FsDir,
    capabilities::{require_table_fs_find, require_table_fs_read},
    roadmap11::known_exts,
};

/// One text match returned by [`FsDir::find_grep`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindMatch {
    /// Relative file path using `/` separators.
    pub path: String,
    /// One-based line number.
    pub line: u32,
    /// Matching line text without its trailing line break.
    pub text: String,
}

/// Bounded grep result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindGrepResult {
    /// Matches retained within the requested bound.
    pub matches: Vec<FindMatch>,
    /// True when more matches existed after `matches` reached `max`.
    pub truncated: bool,
}

/// Bounded glob result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindGlobResult {
    /// Relative paths retained within the requested bound.
    pub paths: Vec<String>,
    /// True when more paths existed after `paths` reached `max`.
    pub truncated: bool,
}

impl FsDir {
    /// Regex-search string leaves under this directory.
    ///
    /// Requires both `fs/read` and `find`. The optional glob filters relative
    /// paths before any file is read. Returned matches are bounded by `max`;
    /// truncation is reported through [`FindGrepResult::truncated`].
    pub fn find_grep(
        &self,
        cx: &mut Cx,
        pattern: &str,
        glob: Option<&str>,
        max: usize,
    ) -> Result<FindGrepResult> {
        require_table_fs_read(cx)?;
        require_table_fs_find(cx)?;
        let regex =
            Regex::new(pattern).map_err(|err| Error::Eval(format!("table/fs: regex {err}")))?;
        let glob = glob.map(glob_matcher).transpose()?;
        let mut result = FindGrepResult {
            matches: Vec::new(),
            truncated: false,
        };
        let mut seen_dirs = self.initial_seen_dirs()?;
        self.walk_search_entries(&mut seen_dirs, |path, rel, is_dir| {
            if is_dir || !glob_matches(glob.as_ref(), rel) {
                return Ok(false);
            }
            let Some(ext) = known_search_ext(path) else {
                return Ok(false);
            };
            let text = self.search_text_for_path(cx, path, ext)?;
            for (line, line_text) in text.lines().enumerate() {
                if regex.is_match(line_text) {
                    if result.matches.len() >= max {
                        result.truncated = true;
                        return Ok(true);
                    }
                    result.matches.push(FindMatch {
                        path: rel.to_owned(),
                        line: (line + 1).try_into().unwrap_or(u32::MAX),
                        text: line_text.to_owned(),
                    });
                }
            }
            Ok(false)
        })?;
        Ok(result)
    }

    /// Glob relative paths under this directory without reading file contents.
    ///
    /// Requires both `fs/read` and `find`. Returned paths are bounded by `max`;
    /// truncation is reported through [`FindGlobResult::truncated`].
    pub fn find_glob(&self, cx: &mut Cx, pattern: &str, max: usize) -> Result<FindGlobResult> {
        require_table_fs_read(cx)?;
        require_table_fs_find(cx)?;
        let glob = glob_matcher(pattern)?;
        let mut result = FindGlobResult {
            paths: Vec::new(),
            truncated: false,
        };
        let mut seen_dirs = self.initial_seen_dirs()?;
        self.walk_search_entries(&mut seen_dirs, |_path, rel, _is_dir| {
            if glob.is_match(rel) {
                if result.paths.len() >= max {
                    result.truncated = true;
                    return Ok(true);
                }
                result.paths.push(rel.to_owned());
            }
            Ok(false)
        })?;
        Ok(result)
    }

    fn initial_seen_dirs(&self) -> Result<BTreeSet<PathBuf>> {
        let mut seen = BTreeSet::new();
        seen.insert(
            std::fs::canonicalize(self.root_path())
                .map_err(|err| Error::Eval(format!("table/fs: find root {err}")))?,
        );
        Ok(seen)
    }

    fn walk_search_entries<F>(&self, seen_dirs: &mut BTreeSet<PathBuf>, mut visit: F) -> Result<()>
    where
        F: FnMut(&Path, &str, bool) -> Result<bool>,
    {
        self.walk_search_dir(self.root_path(), "", seen_dirs, &mut visit)
            .map(|_| ())
    }

    fn walk_search_dir<F>(
        &self,
        dir: &Path,
        rel_prefix: &str,
        seen_dirs: &mut BTreeSet<PathBuf>,
        visit: &mut F,
    ) -> Result<bool>
    where
        F: FnMut(&Path, &str, bool) -> Result<bool>,
    {
        let entries = sorted_entries(dir)?;
        for (name, path) in entries {
            if name.starts_with('.') {
                continue;
            }
            self.ensure_internal_path(&path)?;
            let rel = if rel_prefix.is_empty() {
                name
            } else {
                format!("{rel_prefix}/{name}")
            };
            let metadata = std::fs::metadata(&path)
                .map_err(|err| Error::Eval(format!("table/fs: find metadata {err}")))?;
            if metadata.is_dir() {
                if visit(&path, &rel, true)? {
                    return Ok(true);
                }
                let canonical = std::fs::canonicalize(&path)
                    .map_err(|err| Error::Eval(format!("table/fs: find path {err}")))?;
                if seen_dirs.insert(canonical)
                    && self.walk_search_dir(&path, &rel, seen_dirs, visit)?
                {
                    return Ok(true);
                }
            } else if metadata.is_file() && visit(&path, &rel, false)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn search_text_for_path(&self, cx: &mut Cx, path: &Path, ext: &str) -> Result<String> {
        let expr = self.read_leaf_path(cx, path, ext)?;
        match expr {
            Expr::String(text) => Ok(text),
            Expr::Extension { payload, .. } => match *payload {
                Expr::String(text) => Ok(text),
                _ => Err(Error::Eval(format!(
                    "table/fs: find expects string payload at {}",
                    relative_path(self.root_path(), path)?
                ))),
            },
            _ => Err(Error::Eval(format!(
                "table/fs: find expects string leaf at {}",
                relative_path(self.root_path(), path)?
            ))),
        }
    }
}

fn sorted_entries(dir: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut entries = BTreeMap::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|err| Error::Eval(format!("table/fs: find read_dir {err}")))?
    {
        let entry = entry.map_err(|err| Error::Eval(format!("table/fs: find entry {err}")))?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| Error::Eval("table/fs: find path is not utf-8".to_owned()))?
            .to_owned();
        entries.insert(name, entry.path());
    }
    Ok(entries)
}

fn glob_matcher(pattern: &str) -> Result<GlobMatcher> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map_err(|err| Error::Eval(format!("table/fs: glob {err}")))
        .map(|glob| glob.compile_matcher())
}

fn glob_matches(glob: Option<&GlobMatcher>, rel: &str) -> bool {
    glob.is_none_or(|glob| glob.is_match(rel))
}

fn known_search_ext(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    known_exts().into_iter().find(|known| *known == ext)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| Error::Eval(format!("table/fs: find relative path {err}")))?;
    Ok(rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
