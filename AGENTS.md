# AGENTS.md

jqc reads JSONC. Everything else is jq: jqc converts JSONC to JSON and runs the `jq` on `PATH`. It never computes jq results itself. User-facing behavior is in README.md.

## Commands

```bash
make                          # cargo check, test, clippy, fmt --check (what CI runs)
cargo test                    # all tests
cargo test --test cli         # E2E tests only
cargo test <name>             # tests whose name contains <name>
cargo clippy --all-targets --all-features
cargo fmt --all
cargo build && PATH="$PWD/target/debug:$PATH" bash scripts/jq-compat.sh   # jqc vs jq
```

- Toolchain and tools are pinned in `mise.toml` (Rust, prek, cargo-dist). Run `mise install` if they are missing.
- The tests and `scripts/jq-compat.sh` need `jq` on `PATH`. CI uses jq 1.8.2 (`JQ_VERSION` in `.github/workflows/jq-compat.yml`).
- The prek pre-commit hook (`.pre-commit-config.yml`) runs `cargo fmt --check`, `cargo check`, `cargo clippy` and `cargo test` on every commit.

## Architecture

| Module | Role |
|---|---|
| `main.rs` | Routes a command to filter mode, edit mode (`--edit` / `--in-place`) or `fmt`. Feeds converted input to jq, maps exit statuses, prints output (exit 141 on a closed pipe). Holds `HELP` and the edit-mode option checks (`EDIT_REJECTED_OPTIONS`, `EDIT_REJECTED_SHORT`). |
| `args.rs` | Splits argv the way jq does: short-option clusters, options that take values, `--args`, `--`. It records which arguments are input files and `--slurpfile` files, and leaves the rest for jq unchanged. |
| `jsonc.rs` | JSONC to JSON, one JSON text per top-level value. Numbers keep their text and duplicate keys merge the way jq merges them. `convert_prefix` returns the values before the first error, because jq prints the values it read before failing. |
| `jq.rs` | Runs jq: `spawn` (filter mode, jq writes to jqc's stdout) and `output` (captures stdout; can remove environment variables). |
| `patch.rs` | Edit mode's write-back. It diffs the original value against jq's result and writes only the differences into the CST (jsonc-parser `cst`), so comments and formatting stay. |
| `color.rs` | jqc's own colorizer, used for `fmt` and `--edit` output only. It reads tokens with jsonc-parser's `Scanner`, colors them the way `jq -C` does, and reads `JQ_COLORS` with jq 1.8.2's rules plus a 9th field for comments. |

### Design constraints

- **Filter mode is jq.** Only the input is converted; options and the filter go to jq unchanged. jq's own exit status and messages stand. jqc reports its own errors as `Error: ...` and uses status 2 (usage or system error) or 5 (input it can't parse).
- **Broken input**: jqc feeds the values before the error, then a `]` that jq always rejects. That way jq decides whether the error is reached, and jqc prints its parse message only when jq exits 5.
- **Edit mode**, for each document: `jsonc::convert` → jq canonicalizes the source (`CANONICALIZE` in `edit_document`; NaN becomes a marker string) → jq runs the user's filter with `-c -M` → `patch::write_back(text, source, result)`. Input and result must be exactly one value each. A failure writes nothing.
- **Colors**: in filter mode jq colors its own output. `fmt` and `--edit` must stay byte-identical to `jq -C` for the same tokens. `color.rs`'s tests and `tests/cli.rs` check this against jq 1.8.2's output.
- **Differences from jq** are listed in README.md ("Differences from jq") and in `tests/fixtures/jq-compat/cases.json`. A change that adds or removes a difference updates both.

## Testing

- Unit tests live in each module (`#[cfg(test)]`). E2E tests are in `tests/cli.rs` (`assert_cmd`, `predicates`), with fixtures in `tests/fixtures/`.
- E2E tests check jqc's own behavior and don't hardcode jq's output, except for color bytes. Tests that compare colors use `jqc_default_colors()`, which removes `JQ_COLORS` and `NO_COLOR` from the environment.
- `tests/fixtures/jq-compat/cases.json` is the list of differences from jq plus a few checks that jqc passes jq's output through unchanged. Run it with `scripts/jq-compat.sh`. A case with a `jqc` or `jqc_status` expectation needs a `note`.

## Commits and releases

- Conventional Commits (`feat:`, `fix:`, `docs:`, `build:`, `refactor:`, `test:`, `chore:`; `!` for breaking changes). release-please builds the CHANGELOG and release PRs from them (`release-please-config.json`).
- Releases are built and published by cargo-dist (`dist-workspace.toml`, generated `.github/workflows/release.yml`). After changing dist config, run `dist generate`, and never edit `release.yml` by hand.
- PRs are squash-merged into `main`.
