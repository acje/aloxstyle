# AloxStyle - Robust Oxidation

See [AloxStyle](aloxstyle.md).

See [Things That Are Bugs](bugs.md).

## Implementations and adopters

- [alumina-proxy](https://github.com/acje/alumina-proxy)
- [canary](https://github.com/acje/canary)

## Lint configuration

AloxStyle ships a copy-in lint configuration:

| File | Role |
| --- | --- |
| `cargo-lints.toml` | The complete lint table: `[workspace.lints.rust]` and `[workspace.lints.clippy]`. |
| `clippy.toml` | Clippy driver configuration: test exemptions for `unwrap` and `expect`. |

Cargo has no remote include mechanism, so adopters copy both files and keep the
copies as the source of truth in their own repositories. Neither file carries
comments; AloxStyle prohibits plain comments, and the rationale lives here.

Copy-in steps:

- **Workspace layout.** Append the contents of `cargo-lints.toml` to the
  workspace root `Cargo.toml`; its tables are already named
  `[workspace.lints.rust]` and `[workspace.lints.clippy]`. Add `[lints]` with
  `workspace = true` to every member `Cargo.toml`.
- **Single-crate layout.** Rename the two tables to `[lints.rust]` and
  `[lints.clippy]` in the crate's `Cargo.toml`. No opt-in table is needed.
- **`clippy.toml` placement.** Copy it next to the manifest being checked: the
  workspace root for a workspace, the crate directory for a single crate. Clippy
  reads `clippy.toml` from the checked crate's manifest directory and its
  ancestors. Without it, `unwrap_used` and `expect_used` also fire in test code.
- **Run** `cargo clippy --all-targets`.

`allow-unwrap-in-tests = true` and `allow-expect-in-tests = true` keep
`unwrap()` and `expect()` silent in `#[cfg(test)]` modules and integration
tests, where a panicking assertion is the expected failure mode.

### Lint levels

Three lints are `deny`, matching the style's wording: `unused_must_use` in
`[workspace.lints.rust]`, plus `unwrap_used` and `expect_used` in
`[workspace.lints.clippy]`. Every other lint is `warn`. A repository that adopts
the table with existing violations will fail to build until it fixes them or
silences each site deliberately.

Justified `expect()` calls are marked with a per-site expectation carrying a
reason, for example `#[expect(clippy::expect_used, reason = "construction
proves the resolver set is non-empty")]`. `allow_attributes_without_reason`
keeps `#[allow]` exceptions reasoned as well.

### Deliberate omissions

- `arithmetic_side_effects` is not enabled: it flags every arithmetic operation
  on integers, including test code and values whose range is fixed by
  construction, so it does not separate unguarded overflow from arithmetic the
  style accepts.
- `as_conversions`, `wildcard_enum_match_arm`, `allow_attributes`, and the
  `pedantic`, `nursery`, `cargo`, and `restriction` groups are not enabled: they
  fire on constructs the style permits, or they are too noisy to serve as the
  de-facto implementation.
- `disallowed-methods` and `disallowed-types` are not set in `clippy.toml`:
  library-specific policy belongs to the project, not to the style.
- The plain line and block comment prohibition is not lint-enforceable; it is
  served by a separate `comment-free` tool.
- The unsafe code policy is pending, so neither file sets `unsafe_code`.
  `undocumented_unsafe_blocks` is `warn` in the meantime.

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
