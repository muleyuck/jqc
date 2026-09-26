# jq compatibility cases

`tests/jq_compat.rs` runs every case in `cases.json` through jqc and compares the result with jq's output recorded in `expected.json`. `cargo test` never runs jq itself.

## Add a case

1. Add the case to `cases.json`.

   | Field | Required | Meaning |
   |---|---|---|
   | `name` | yes | Unique name of the case |
   | `args` | yes | Arguments passed unchanged to both jq and jqc |
   | `stdin` | no | Text written to stdin. Empty when omitted |
   | `files` | no | File name → content. The files are created in a temporary directory, and both tools run there |
   | `compare` | no | `"text"` (default) compares stdout exactly. `"value"` compares stdout as a JSON value; use it for edit expressions, because jqc keeps the source formatting |
   | `known_difference` | no | Issue number of a known difference from jq |

   Both modes also compare the exit status. stderr is never compared.

2. Regenerate `expected.json` with `scripts/jq-compat.sh`. It needs the jq version pinned as `JQ_VERSION` in `.github/workflows/jq-compat.yml`. Without jq, push the change instead: the `jq-compat` workflow fails and uploads the regenerated file as the `jq-compat-expected` artifact. Download it and commit it here.
3. Run `cargo test --test jq_compat`. If jqc doesn't match jq yet, open an issue and set its number as the case's `known_difference`.

## Fix a known difference

Once jqc matches jq, the test fails and asks you to remove the case's `known_difference`. Remove it in the same PR as the fix.

## When jq is updated

Renovate opens a PR that bumps `JQ_VERSION`. If jq's behavior changed, the `jq-compat` workflow fails in that PR: take the regenerated `expected.json` from its artifact, then fix or file any new differences. jq's added or removed options are not checked automatically, so read the release notes in the PR as well.
