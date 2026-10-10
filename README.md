[![unit-test](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml/badge.svg)](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml)
![Software License](https://img.shields.io/badge/license-MIT-brightgreen.svg?style=flat-square)
[![Release](https://img.shields.io/github/release/muleyuck/jqc.svg)](https://github.com/muleyuck/jqc/releases/latest)

# jqc

**jq for JSONC.** jqc reads JSONC. Everything else is jq.

jqc turns JSONC (comments, trailing commas, single-quoted strings) into JSON and hands it to the `jq` you have installed. `jq` alone stops with a parse error on files like VS Code `settings.json`, `tsconfig.json`, `deno.jsonc` or `biome.jsonc`, because they contain comments. jqc also edits those files without losing the comments.

![demo](https://github.com/user-attachments/assets/24711d01-76b0-4a37-a3ed-e13a90a62696)

## Install

**Homebrew** (also installs jq)

```bash
brew install muleyuck/tap/jqc
```

**Shell script (macOS / Linux)** (install jq separately)

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/muleyuck/jqc/releases/latest/download/jqc-installer.sh | sh
```

**Cargo** (install jq separately)

```bash
cargo install --git https://github.com/muleyuck/jqc
```

jq is needed for filters and `--edit`; `fmt` does not need it. Get jq from <https://jqlang.org/download/>. Without it, jqc exits with status 2:

```
Error: jq not found: jqc runs filters with jq. Install jq (https://jqlang.org/download/) and make sure it is on PATH
```

## Filter

Use jqc exactly as you use jq. Options and filters are described in the [jq manual](https://jqlang.org/manual/).

```console
$ cat tsconfig.json
{
  // Compiler settings
  "compilerOptions": {
    "target": "ES2022", /* output level */
    "strict": true,
  },
}
$ jqc '.compilerOptions.target' tsconfig.json
"ES2022"
$ jqc -c '.compilerOptions' - < tsconfig.json
{"target":"ES2022","strict":true}
$ jqc -r '.[] | .name' <<'EOF'
[
  {"name": "core"}, // always on
  {'name': 'auth'},
]
EOF
core
auth
```

- Input files, `-` (standard input) and `--slurpfile` files are converted from JSONC. Input of `--rawfile` and `-R` is passed to jq as it is.
- `--help` shows jqc's help; `-h` shows jq's help. `--version` shows the versions of both jqc and jq.

## `fmt`

`jqc fmt [--in-place] [file]` validates JSONC and prints it with its comments kept. It does not need jq. It exits with status 5 when the input cannot be parsed (2 when the file cannot be read), so it works as a pre-commit check.

```console
$ jqc fmt tsconfig.json
{
  // Compiler settings
  "compilerOptions": {
    "target": "ES2022", /* output level */
    "strict": true,
  },
}
$ echo '{"a": }' | jqc fmt
Error: Failed to parse JSONC: Unexpected close brace on line 1 column 7
```

`-` reads standard input. `--in-place` needs a file; it validates the file and writes the same content back. `-C` and `-M` work as in jq and may come before `fmt` (`jqc -C fmt`).

## Edit: `--edit` / `--in-place`

`jqc --edit <filter> [file]` prints the edited document. `jqc --in-place <filter> <files...>` writes the result back to each file. Without either option, jqc runs the filter as jq does (see Differences from jq).

jq computes the result. jqc compares it with the original value and writes only the differences into the original text, so comments, layout and the spelling of unchanged values (number formats, string escapes) stay.

```console
$ cat server.jsonc
{
  // Server settings
  "port": 3000, // default port
  "ratio": 1.0,
  "name": "caf\u00e9"
}
$ jqc --edit '.port = 8080 | .debug = true' server.jsonc
{
  // Server settings
  "port": 8080, // default port
  "ratio": 1.0,
  "name": "caf\u00e9",
  "debug": true
}
```

How the result is written back:

- Object keys keep their order. A key the result no longer has is deleted (every occurrence, if it is duplicated). New keys go at the end. For a duplicated key, the last occurrence is rewritten.
- Arrays of the same length are updated element by element. If elements were only appended, only the new elements are added. Any other change replaces the whole array.
- A value whose type changed is replaced as a whole.

Both the input and the result must be exactly one value each; otherwise jqc exits with status 5. `--edit` takes one file or standard input. `--in-place` takes one or more files (standard input and `-` are not allowed; status 2).

These options cannot be used with `--edit` or `--in-place` (status 2), because an edit reads one document and writes it back in its own format:

- `--null-input`, `--raw-input`, `--slurp`, `--stream`, `--stream-errors`, `--seq`
- `--compact-output`, `--raw-output`, `--raw-output0`, `--join-output`, `--ascii-output`, `--sort-keys`, `--tab`, `--indent`
- `--debug-trace`, `--debug-dump-disasm`, `--build-configuration`, `--run-tests`
- `-n`, `-R`, `-s`, `-c`, `-r`, `-j`, `-a`, `-S`, `-h`, `-V`

For `-c`, `-r` and the like, run the filter without `--edit`.

```console
$ jqc --edit -c '.a = 1' < /dev/null
Error: -c cannot be used with --edit: edits keep the file's own format
```

## Colors

Whether to color is decided as in jq: on for a terminal, `-C` to force, `-M` or `NO_COLOR` to disable.

Colors come from `JQ_COLORS`, with the same rules as jq. jqc adds a 9th field for comments (default `3;90`). jq ignores the 9th field, so you can share one value with jq:

```bash
export JQ_COLORS="0;90:0;39:0;39:0;39:0;32:1;39:1;39:1;34:3;36"
```

The fields are `null:false:true:numbers:strings:arrays:objects:object keys:comments`. An invalid value prints `Failed to set $JQ_COLORS` and falls back to the default colors.

In filter mode, the installed jq colors the output, so colors differ between jq versions (jq 1.7.1, which ships with macOS, places its reset codes differently and limits each `JQ_COLORS` field to 12 characters). `fmt` and `--edit` use the default colors of jq 1.8.2.

## Exit status

- `2`: wrong usage, a file that cannot be read or written, jq not found.
- `5`: input files or standard input that jqc cannot parse as JSONC, or an edit with the wrong number of values. A `--slurpfile` that cannot be parsed gives 2, as in jq.
- Otherwise the exit status of jq (`-e` works as in jq). An unreadable file gives 2 instead of jq's status, unless jq itself exits with 2 or 3 or is killed by a signal.
- `141` when the output pipe is closed; `128 + signal` when jq ends with a signal.

```console
$ echo '[1,2,3]' | jqc -e '.[] | select(. > 5)'; echo $?
4
```

## Differences from jq

jqc converts the input before jq sees it, so some behavior differs from jq.

- `input_filename` is always `"<stdin>"`, even for named files (jq prints the file name): jqc passes the input on standard input.
- jqc reads all input before it runs jq. `tail -f | jqc .` prints nothing and `first(inputs)` waits for the end of the input. jqc reads standard input even when the filter does not use it (`printf x | { jqc -n 1; cat; }` leaves nothing for `cat`). `jqc -n 1 missing.json` exits with 2 (jq: 0).
- When jq stops reading early (`halt`, `first(inputs)`), whether an error in the unread input is reported depends on timing.
- `halt_error(2)` and `halt_error(3)` hide the error of an unreadable input and its exit status 2.
- With `--stream`, events come from the document after duplicate keys are merged. With `--stream` and `--stream-errors`, no events are printed from inside a broken value.
- With `--seq`: JSONC reads NBSP and form feed as whitespace (jq does not). A record that holds a comment and spans files is skipped. An unclosed block comment after a value gets no jq warning. The position in a jq warning points into the converted text. A record that holds only a comment prints nothing (jq warns).
- For broken input, jq's `Unmatched ']'` message is printed above the jqc message. The jqc message has the real position and reason.
- Arguments must be UTF-8.
- Edit: NaN in the edited document becomes null, as in jq's output. Writing the string `"\u0000jqc:NaN"` over a NaN is ignored. A computed value equal to the largest double leaves `Infinity` in place (`.a = infinite`), because jq prints it as `1.7976931348623157e+308`, the same value; a literal `.a = 1.7976931348623157e+308` does replace it.
- Edit: deleting, adding or retyping thousands of elements in one container is slow, and the time grows faster than the container (about 2 seconds to retype 4,000 array elements, about 6 seconds to append 4,000 elements to a 4,000-element array). Changing values of the same kind (numbers to numbers, strings to strings) is fast.
- With an invalid `JQ_COLORS` and several files for `--in-place`, jq warns once per file. With a jq other than 1.8.2, jq's warning and jqc's colors can disagree.

## Development

`make` runs check, test, clippy and fmt. The tests need `jq` on `PATH`. `scripts/jq-compat.sh` and `tests/fixtures/jq-compat/cases.json` list the differences from jq (CI runs them with jq 1.8.2). jqc's own behavior is covered by the E2E tests in `tests/cli.rs`.

## License

MIT
