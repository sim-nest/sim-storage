//! Rust source-size policy gate.

use std::{
    fs,
    path::{Path, PathBuf},
};

const GENERAL_SOFT_LIMIT: usize = 500;
const GENERAL_HARD_LIMIT: usize = 700;
const ENTRYPOINT_SOFT_LIMIT: usize = 150;
const ENTRYPOINT_HARD_LIMIT: usize = 250;

pub fn run() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|err| format!("current dir: {err}"))?;
    let mut files = Vec::new();
    collect_rs_files(&root.join("crates"), &mut files)?;
    collect_rs_files(&root.join("xtask").join("src"), &mut files)?;
    files.sort();

    let mut hard_errors = Vec::new();
    let mut soft_warnings = Vec::new();
    for file in files {
        let lines = count_lines(&file)?;
        let policy = policy_for(&file);
        let display = file
            .strip_prefix(&root)
            .unwrap_or(file.as_path())
            .display()
            .to_string();
        if lines > policy.hard {
            hard_errors.push(format!(
                "file too large: {display} has {lines} lines, limit {}",
                policy.hard
            ));
        } else if lines > policy.soft {
            soft_warnings.push(format!(
                "file above soft target: {display} has {lines} lines, target {}",
                policy.soft
            ));
        }
    }

    for warning in &soft_warnings {
        eprintln!("{warning}");
    }
    if !hard_errors.is_empty() {
        return Err(hard_errors.join("\n"));
    }
    println!(
        "check-file-sizes: OK ({} soft warning(s), 0 hard error(s))",
        soft_warnings.len()
    );
    Ok(())
}

struct Policy {
    soft: usize,
    hard: usize,
}

fn policy_for(path: &Path) -> Policy {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => Policy {
            soft: ENTRYPOINT_SOFT_LIMIT,
            hard: ENTRYPOINT_HARD_LIMIT,
        },
        _ => Policy {
            soft: GENERAL_SOFT_LIMIT,
            hard: GENERAL_HARD_LIMIT,
        },
    }
}

fn collect_rs_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|err| format!("read {}: {err}", path.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", path.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn count_lines(path: &Path) -> Result<usize, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    Ok(text.lines().count())
}
