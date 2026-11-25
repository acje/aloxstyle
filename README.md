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

## License

MIT or Apache-2.0, at your option.
