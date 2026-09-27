# jq compatibility cases

`cases.json` records, for each case, what jq and jqc print for the given arguments and input. `scripts/jq-compat.sh` runs every case through both tools and checks each output against the case's expectation. The `jq-compat` workflow runs it on pull requests with a freshly built jqc and the jq version pinned as `JQ_VERSION` in `.github/workflows/jq-compat.yml`.

## Case format

```json
{
  "name": "raw-output-non-string",
  "args": ["-r","."],
  "stdin": "{\"a\":1}",
  "expect": ["{","  \"a\": 1","}"],
  "jqc": ["{\"a\":1}"],
  "note": "Bug #40: -r prints non-string values on one line instead of pretty-printing them"
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
| `note` | no | Why jqc differs. For a bug, start with `Bug #<issue>:`; for an intended difference, say why (for example, edit mode keeping the source formatting) |

A case with `jqc` or `jqc_status` needs a `note`, so every difference from jq is explained. stderr is never compared. Edit expressions usually pass `-c`, so both tools print one line.

## Add a case

1. Add the case to `cases.json` with what you expect jq to print.
2. Push the change and check the `jq-compat` workflow. A failure shows each tool's actual stdout and exit status, so you can copy jq's output into `expect` if you guessed wrong. If jqc prints something else and it is a bug, open an issue and add `jqc` (and `jqc_status` if needed) with a `note` that starts with `Bug #<issue>:`.

To run the check locally, put the jqc you want to test (for example `target/debug`) and the pinned jq version first in `PATH`, then run `scripts/jq-compat.sh`. Its last line shows which jq and jqc it used.

## When a noted difference disappears

If jqc starts printing what `expect` says in a case that has a `jqc` expectation, the workflow fails and shows the case's `note`:

- For a bug (`Bug #<issue>: ...`), the bug is fixed. Remove the case's `jqc` expectation and `note` in the same PR as the fix.
- For an intended difference, the behavior the note describes is gone. Check whether that change is deliberate before removing them.

## When jq is updated

Renovate opens a PR that bumps `JQ_VERSION`. If jq's output changed, the `jq-compat` workflow reports which `expect` no longer matches: update it, and fix or file any new differences in jqc. jq's added or removed options are not checked automatically, so read the release notes in the PR as well.
