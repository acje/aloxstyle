use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use aloxstyle_lints::{Allowlist, CheckOutcome, Linter};

const MAX_WALK_DEPTH: usize = 64;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
const ALLOWLIST_FILENAME: &str = ".aloxstyle-lints.toml";
/// Well-known generated/build/SCM directories the walk never descends into.
const SKIPPED_DIR_NAMES: &[&str] = &["target", ".git", "node_modules", "vendor", "build", "dist", "out"];

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let outcome = match run(&args) {
        Ok(outcome) => outcome,
        Err(message) => {
            eprintln!("aloxstyle-lints: {message}");
            return ExitCode::from(2);
        }
    };
    match outcome {
        CheckOutcome::Findings(findings) => {
            for finding in &findings {
                println!("{finding}");
            }
            if findings.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        CheckOutcome::Indeterminate(file) => {
            eprintln!(
                "aloxstyle-lints: {} is not parseable; results are indeterminate",
                file.display()
            );
            ExitCode::from(3)
        }
    }
}

/// Parsed command line: the target path and an optional allowlist config file.
struct Cli {
    target: PathBuf,
    allow_config: Option<PathBuf>,
}

fn run(args: &[OsString]) -> Result<CheckOutcome, String> {
    let cli = parse_args(args)?;
    let allowlist = load_allowlist(&cli.target, cli.allow_config.as_deref())?;
    let files = collect_sources(&cli.target)?;
    let mut findings = Vec::new();
    for file in files {
        let source_text = read_source(&file)?;
        match Linter::with_allowlist(file, allowlist.clone()).check(&source_text) {
            CheckOutcome::Findings(found) => findings.extend(found),
            CheckOutcome::Indeterminate(file) => return Ok(CheckOutcome::Indeterminate(file)),
        }
    }
    Ok(CheckOutcome::Findings(findings))
}

/// Resolves and parses the allowlist config file.
///
/// An explicit `--allow-config <file>` is required to exist. Otherwise a
/// `.aloxstyle-lints.toml` in the analyzed tree root is used when present, and
/// an empty allowlist is returned when it is absent.
fn load_allowlist(target: &Path, explicit: Option<&Path>) -> Result<Allowlist, String> {
    let config_path = explicit.map_or_else(
        || auto_allow_config_path(target),
        |path| Some(path.to_path_buf()),
    );
    let Some(config_path) = config_path else {
        return Ok(Allowlist::empty());
    };
    let text = match fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(e) => {
            if explicit.is_some() {
                return Err(format!(
                    "read allowlist {}: {e}",
                    config_path.display()
                ));
            }
            return Ok(Allowlist::empty());
        }
    };
    Allowlist::parse(&text).map_err(|e| format!("{}: {e}", config_path.display()))
}

/// The nearest `.aloxstyle-lints.toml` at or above the analyzed tree root.
fn auto_allow_config_path(target: &Path) -> Option<PathBuf> {
    let mut dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent()?.to_path_buf()
    };
    loop {
        let candidate = dir.join(ALLOWLIST_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn parse_args(args: &[OsString]) -> Result<Cli, String> {
    let mut target: Option<PathBuf> = None;
    let mut allow_config: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let s = arg.to_string_lossy();
        if s == "--allow-config" {
            if allow_config.is_some() {
                return Err(usage().to_string());
            }
            let file = args.get(i + 1).ok_or_else(|| usage().to_string())?;
            allow_config = Some(PathBuf::from(file.as_os_str()));
            i += 2;
        } else if s.starts_with("--") {
            return Err(usage().to_string());
        } else {
            if target.is_some() {
                return Err(usage().to_string());
            }
            target = Some(PathBuf::from(arg.as_os_str()));
            i += 1;
        }
    }
    let target = target.ok_or_else(|| usage().to_string())?;
    Ok(Cli { target, allow_config })
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

fn collect_sources(target: &Path) -> Result<Vec<PathBuf>, String> {
    if target.is_dir() {
        return collect_dir_sources(target, 0);
    }
    if target.is_file() {
        if target.extension().is_some_and(|e| e == "rs") {
            return Ok(vec![target.to_path_buf()]);
        }
        return Err(format!(
            "{} is not a Rust source file (.rs); pass a .rs file or a directory",
            target.display()
        ));
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
            if is_skipped_generated_dir(&path) {
                continue;
            }
            files.extend(collect_dir_sources(&path, depth + 1)?);
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(files)
}

fn is_skipped_generated_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| SKIPPED_DIR_NAMES.contains(&name))
}

const fn usage() -> &'static str {
    "usage: aloxstyle-lints <path-or-file> [--allow-config <file>]
exit codes: 0 = no findings, 1 = findings, 2 = usage/IO error, 3 = indeterminate (unparseable source)"
}
