# jq compatibility cases

`scripts/jq-compat.sh` runs every case in `cases.json` through both jq and jqc and reports where jqc's output differs from jq's. The `jq-compat` workflow runs it on pull requests with a freshly built jqc and the jq version pinned as `JQ_VERSION` in `.github/workflows/jq-compat.yml`.

## Add a case

1. Add the case to `cases.json`.

   | Field | Required | Meaning |
   |---|---|---|
   | `name` | yes | Unique name of the case |
   | `args` | yes | Arguments passed unchanged to both jq and jqc |
   | `stdin` | no | Text written to stdin. Empty when omitted |
   | `files` | no | File name → content. The files are created in a temporary directory, and both tools run there |
   | `compare` | no | `"text"` (default) compares stdout exactly. `"value"` lets jq read both outputs and compares them with jq's `==`; use it for edit expressions, because jqc keeps the source formatting |
   | `known_difference` | no | Issue number of a known difference from jq |

   Both modes also compare the exit status. stderr is never compared.

2. Push the change and check the `jq-compat` workflow. If jqc doesn't match jq yet, open an issue and set its number as the case's `known_difference`.

To run the comparison locally, put the jqc you want to test (for example `target/debug`) and the pinned jq version first in `PATH`, then run `scripts/jq-compat.sh`. Its last line shows which jq and jqc it used.

## Fix a known difference

Once jqc matches jq, the workflow fails and asks you to remove the case's `known_difference`. Remove it in the same PR as the fix.

## When jq is updated

Renovate opens a PR that bumps `JQ_VERSION`. If jq's behavior changed, the `jq-compat` workflow fails in that PR: fix or file any new differences. jq's added or removed options are not checked automatically, so read the release notes in the PR as well.
