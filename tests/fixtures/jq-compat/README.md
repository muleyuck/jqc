# jq compatibility cases

`cases.json` records, for each case, what jq and jqc print for the given arguments and input. `scripts/jq-compat.sh` runs every case through both tools and checks each output against the case's expectation. The `jq-compat` workflow runs it on pull requests with a freshly built jqc and the jq version pinned as `JQ_VERSION` in `.github/workflows/jq-compat.yml`.

## Case format

```json
{
  "name": "identity-keeps-key-order",
  "args": ["."],
  "stdin": "{\"b\":1,\"a\":2}",
  "expect": ["{","  \"b\": 1,","  \"a\": 2","}"],
  "jqc": ["{","  \"a\": 2,","  \"b\": 1","}"],
  "known_difference": 39
}
```

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | Unique name of the case |
| `args` | yes | Arguments passed unchanged to both jq and jqc |
| `stdin` | no | Text written to stdin. Empty when omitted |
| `files` | no | File name → content. The files are created in a temporary directory, and both tools run there |
| `expect` | yes | What jq prints to stdout, one array element per line. `[]` means no output |
| `status` | no | jq's exit status. `0` when omitted |
| `jqc` | no | What jqc prints, when it differs from `expect`. When omitted, jqc must print `expect` |
| `jqc_status` | no | jqc's exit status, when it differs from `status` |
| `known_difference` | no | Issue number, when jqc's difference from jq is a bug |
| `note` | no | Why jqc differs, when the difference is intended (for example, edit mode keeping the source formatting) |

A case with `jqc` or `jqc_status` needs a `known_difference` or a `note`, so every difference from jq is explained. stderr is never compared. Edit expressions usually pass `-c`, so both tools print one line.

## Add a case

1. Add the case to `cases.json`, writing down what jq prints (run it with the pinned jq version).
2. Push the change and check the `jq-compat` workflow. If jqc prints something else and it is a bug, open an issue and add `jqc` (and `jqc_status` if needed) with the issue number as `known_difference`.

To run the check locally, put the jqc you want to test (for example `target/debug`) and the pinned jq version first in `PATH`, then run `scripts/jq-compat.sh`. Its last line shows which jq and jqc it used.

## Fix a known difference

Once jqc prints what `expect` says, the workflow fails and asks you to remove the case's `jqc` expectation and `known_difference`. Remove them in the same PR as the fix.

## When jq is updated

Renovate opens a PR that bumps `JQ_VERSION`. If jq's output changed, the `jq-compat` workflow reports which `expect` no longer matches: update it, and fix or file any new differences in jqc. jq's added or removed options are not checked automatically, so read the release notes in the PR as well.
