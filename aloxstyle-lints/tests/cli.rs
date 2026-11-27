use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aloxstyle-lints"))
}

fn make_fixture() -> Result<PathBuf, Box<dyn std::error::Error>> {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = env::temp_dir().join(format!(
        "aloxstyle-lints-fixture-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("fx.rs"), "pub fn fx() {}\n")?;
    fs::write(dir.join("lib.rs"), "/// a doc comment\npub fn lib() {}\n")?;
    Ok(dir)
}

#[test]
fn cli_returns_zero_for_clean_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let dir = make_fixture()?;
    let output = Command::new(bin_path()).arg(&dir).output()?;
    assert!(output.status.success(), "exit should be 0, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[test]
fn cli_returns_zero_for_single_clean_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = make_fixture()?;
    let file = dir.join("fx.rs");
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert!(output.status.success(), "exit should be 0, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[test]
fn cli_no_arg_usage_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new(bin_path()).output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "no-arg invocation should exit 2, got {output:?}"
    );
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("usage"), "expected usage on stderr, got {stderr}");
    Ok(())
}

#[test]
fn cli_bad_path_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    let dir = make_fixture()?;
    let missing = dir.join("does-not-exist");
    let output = Command::new(bin_path()).arg(&missing).output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "bad path should exit 2, got {output:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn cli_skips_symlink_cycle_dir() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let dir = make_fixture()?;
    symlink(&dir, dir.join("loop"))?;
    let output = Command::new(bin_path()).arg(&dir).output()?;
    assert!(
        output.status.success(),
        "symlink cycle should not crash the walk, got {output:?}"
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn cli_non_utf8_arg_does_not_panic() -> Result<(), Box<dyn std::error::Error>> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let non_utf8 = OsStr::from_bytes(b"/tmp/aloxstyle-lints-\xff");
    let output = Command::new(bin_path()).arg(non_utf8).output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "non-UTF8 arg should exit 2, not panic, got {output:?}"
    );
    Ok(())
}

fn write_source(name: &str, content: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    static SRC_COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = env::temp_dir().join(format!(
        "aloxstyle-lints-src-{}-{}",
        std::process::id(),
        SRC_COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir)?;
    let file = dir.join(name);
    fs::write(&file, content)?;
    Ok(file)
}

#[test]
fn cli_flags_plain_line_comment() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "// foo\npub fn fx() {}\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(output.status.code(), Some(1), "expected exit 1, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains(":1:1:"),
        "finding line:col on stdout, got {stdout}"
    );
    assert!(
        stdout.contains("plain line comment"),
        "message on stdout, got {stdout}"
    );
    Ok(())
}

#[test]
fn cli_does_not_flag_doc_and_safety() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source(
        "fx.rs",
        "//! inner doc\n/// outer doc\n// SAFETY: sound\npub unsafe fn fx() {}\n",
    )?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert!(output.status.success(), "expected exit 0, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[test]
fn cli_oversized_source_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    static SRC_COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = env::temp_dir().join(format!(
        "aloxstyle-lints-big-{}-{}",
        std::process::id(),
        SRC_COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir)?;
    let file = dir.join("big.rs");
    let big = vec![b' '; (1024 * 1024) + 1];
    fs::write(&file, &big)?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "oversize should exit 2, got {output:?}"
    );
    Ok(())
}

#[test]
fn cli_bare_unsafe_exits_one() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "pub fn fx() { unsafe {} }\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(output.status.code(), Some(1), "expected exit 1, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("unsafe block"),
        "expected unsafe-block finding, got {stdout}"
    );
    assert!(
        stdout.contains("// SAFETY:"),
        "message references the marker, got {stdout}"
    );
    Ok(())
}

#[test]
fn cli_safety_marker_unsafe_exits_zero() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source(
        "fx.rs",
        "pub fn fx() {\n// SAFETY: sound\nlet d = unsafe {};\n}\n",
    )?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert!(output.status.success(), "expected exit 0, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[test]
fn cli_unsafe_impl_without_marker_exits_one() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "trait T {}\nstruct S;\nunsafe impl T for S {}\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(output.status.code(), Some(1), "expected exit 1, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("unsafe impl"),
        "expected unsafe-impl finding, got {stdout}"
    );
    Ok(())
}

#[test]
fn cli_unparseable_source_exits_three() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "fn broken( {\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(
        output.status.code(),
        Some(3),
        "unparseable source should exit 3, got {output:?}"
    );
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("indeterminate"),
        "expected indeterminate on stderr, got {stderr}"
    );
    assert!(
        stderr.contains("fx.rs"),
        "expected the offending file name on stderr, got {stderr}"
    );
    Ok(())
}

#[test]
fn cli_flags_underflow_without_allowlist() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(output.status.code(), Some(1), "expected exit 1, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("underflow"),
        "expected underflow finding, got {stdout}"
    );
    Ok(())
}

#[test]
fn cli_allow_config_suppresses_underflow() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n")?;
    let dir = file.parent().expect("has parent");
    let allow = dir.join("allow.toml");
    fs::write(
        &allow,
        "[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nsnippet = \"a - b\"\nreason = \"test site is sized so b never exceeds a\"\n",
    )?;
    let output = Command::new(bin_path())
        .arg(&file)
        .arg("--allow-config")
        .arg(&allow)
        .output()?;
    assert!(output.status.success(), "expected exit 0, got {output:?}");
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    Ok(())
}

#[test]
fn cli_allow_config_missing_reason_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n")?;
    let dir = file.parent().expect("has parent");
    let allow = dir.join("allow.toml");
    fs::write(
        &allow,
        "[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nsnippet = \"a - b\"\n",
    )?;
    let output = Command::new(bin_path())
        .arg(&file)
        .arg("--allow-config")
        .arg(&allow)
        .output()?;
    assert_eq!(output.status.code(), Some(2), "expected exit 2, got {output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("no `reason`"),
        "expected missing-reason error, got {stderr}"
    );
    Ok(())
}

#[test]
fn cli_allow_config_missing_file_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("fx.rs", "pub fn fx() {}\n")?;
    let missing = file
        .parent()
        .expect("has parent")
        .join("does-not-exist.toml");
    let output = Command::new(bin_path())
        .arg(&file)
        .arg("--allow-config")
        .arg(&missing)
        .output()?;
    assert_eq!(output.status.code(), Some(2), "expected exit 2, got {output:?}");
    Ok(())
}

#[test]
fn cli_non_rs_direct_file_exits_two() -> Result<(), Box<dyn std::error::Error>> {
    let file = write_source("note.txt", "not a rust source\n")?;
    let output = Command::new(bin_path()).arg(&file).output()?;
    assert_eq!(
        output.status.code(),
        Some(2),
        "a non-.rs direct-file target is a scope error, got {output:?}"
    );
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("not a Rust source"),
        "expected a clear not-a-rust message on stderr, got {stderr}"
    );
    Ok(())
}

#[test]
fn cli_dir_walk_skips_generated_dirs() -> Result<(), Box<dyn std::error::Error>> {
    static DIR_COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = env::temp_dir().join(format!(
        "aloxstyle-lints-walk-{}-{}",
        std::process::id(),
        DIR_COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(dir.join("target"))?;
    fs::write(dir.join("main.rs"), "pub fn main() {}\n")?;
    fs::write(dir.join("target").join("out.rs"), "// generated\npub fn out() {}\n")?;
    let output = Command::new(bin_path()).arg(&dir).output()?;
    assert!(
        output.status.success(),
        "walk must skip generated dirs, got {output:?}"
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.is_empty(), "expected no findings, got {stdout}");
    let out_file = dir.join("target").join("out.rs");
    let direct = Command::new(bin_path()).arg(&out_file).output()?;
    assert_eq!(
        direct.status.code(),
        Some(1),
        "direct target of a generated file still flags it"
    );
    Ok(())
}
