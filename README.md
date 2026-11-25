# AloxStyle - Robust Oxidation

- [AloxStyle](aloxstyle.md) — the style.
- [Things That Are Bugs](bugs.md) — the bug taxonomy.

Adopters: [alumina-proxy](https://github.com/acje/alumina-proxy), [canary](https://github.com/acje/canary).

## Lint configuration

Copy-in (Cargo has no remote include; neither file carries comments — the rationale lives here):

| File | Role |
| --- | --- |
| `cargo-lints.toml` | Complete table: `[workspace.lints.rust]` + `[workspace.lints.clippy]`. |
| `clippy.toml` | Test exemptions for `unwrap` and `expect`. |

- **Workspace** — append the tables to the workspace root `Cargo.toml`; add `[lints]` with `workspace = true` to every member.
- **Single crate** — rename the two tables to `[lints.rust]` / `[lints.clippy]`.
- **`clippy.toml`** — copy next to the checked manifest (workspace root, or crate dir). Without it, `unwrap_used`/`expect_used` also fire in test code.
- **Run** `cargo clippy --all-targets`.

`allow-unwrap-in-tests` / `allow-expect-in-tests` keep `unwrap()`/`expect()` silent in `#[cfg(test)]` modules and integration tests, where a panicking assertion is the expected failure mode.

### Levels

Eight lints are `deny` — the rest `warn`:

- `[workspace.lints.rust]`: `unused_must_use`, `unsafe_op_in_unsafe_fn`.
- `[workspace.lints.clippy]`: `unwrap_used`, `expect_used`, `allow_attributes_without_reason`, plus production panic markers `dbg_macro`, `todo`, `unimplemented`.

A repo adopting the table with existing violations fails to build until it fixes them or silences each site deliberately — e.g. `#[expect(clippy::expect_used, reason = "…")]`, while `allow_attributes_without_reason` keeps `#[allow]` exceptions reasoned too.

### Deliberate omissions

- `arithmetic_side_effects`, `as_conversions`, `wildcard_enum_match_arm`, `allow_attributes`, and the `nursery`/`cargo`/`restriction` groups are not enabled — they fire on constructs the style permits or are too noisy.
- The `pedantic` group is not enabled wholesale; five of its lints are adopted as `warn`: `match_wild_err_arm`, `option_option`, `unchecked_time_subtraction`, `manual_instant_elapsed`, `large_types_passed_by_value`.
- `disallowed-methods` / `disallowed-types` are unset — library-specific policy belongs to the project.
- The plain-comment prohibition is not lint-enforceable (served by a separate `comment-free` tool). Unsafe policy is pending, so `unsafe_code` is unset; `undocumented_unsafe_blocks` is `warn`, and `// SAFETY:` is the one comment form the style permits (see [Explain through structure and contracts](aloxstyle.md#explain-through-structure-and-contracts)).

## License

MIT or Apache-2.0, at your option.
