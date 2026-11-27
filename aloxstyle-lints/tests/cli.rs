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
