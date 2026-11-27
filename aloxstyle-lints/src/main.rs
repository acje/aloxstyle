use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use aloxstyle_lints::{Finding, Linter};

const MAX_WALK_DEPTH: usize = 64;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let findings = match run(&args) {
        Ok(findings) => findings,
        Err(message) => {
            eprintln!("aloxstyle-lints: {message}");
            return ExitCode::from(2);
        }
    };
    for finding in &findings {
        println!("{finding}");
    }
    if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run(args: &[OsString]) -> Result<Vec<Finding>, String> {
    let target = parse_target(args)?;
    let files = collect_sources(&target)?;
    let mut findings = Vec::new();
    for file in files {
        let source_text = read_source(&file)?;
        findings.extend(Linter::new(file).check(&source_text));
    }
    Ok(findings)
}

fn read_source(file: &Path) -> Result<String, String> {
    let meta = fs::metadata(file).map_err(|e| format!("metadata {}: {e}", file.display()))?;
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "{} exceeds {MAX_SOURCE_BYTES} bytes",
            file.display()
        ));
    }
    fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file.display()))
}

fn parse_target(args: &[OsString]) -> Result<PathBuf, String> {
    match args {
        [single] => Ok(PathBuf::from(single.as_os_str())),
        _ => Err(usage().to_string()),
    }
}

fn collect_sources(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_dir() {
        return collect_dir_sources(target, 0);
    }
    if target.is_file() {
        if target.extension().is_some_and(|e| e == "rs") {
            return Ok(vec![target.to_path_buf()]);
        }
        return Ok(Vec::new());
    }
    Err(format!("no such path: {}", target.display()))
}

fn collect_dir_sources(dir: &Path, depth: usize) -> Result<Vec<PathBuf>, String> {
    if depth > MAX_WALK_DEPTH {
        return Err(format!(
            "directory tree deeper than {MAX_WALK_DEPTH} at {}",
            dir.display()
        ));
    }
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            files.extend(collect_dir_sources(&path, depth + 1)?);
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(files)
}

const fn usage() -> &'static str {
    "usage: aloxstyle-lints <path-or-file>"
}
