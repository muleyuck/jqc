[![unit-test](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml/badge.svg)](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml)
![Software License](https://img.shields.io/badge/license-MIT-brightgreen.svg?style=flat-square)
[![Release](https://img.shields.io/github/release/muleyuck/jqc.svg)](https://github.com/muleyuck/jqc/releases/latest)

# jqc

**jq for JSONC.** Query and edit JSON-with-comments files (`tsconfig.json`, VS Code `settings.json`, `deno.jsonc`, `biome.jsonc`) without losing the comments.

`jq` stops at the first comment. jqc reads the file, then everything works as in jq.

![demo](tapes/demo.gif)

## Edit without losing comments

```console
$ cat config.jsonc
{
  // Server settings
  "host": "localhost",
  "port": 3000, // default port
  /* Feature flags */
  "debug": false
}
$ jqc --edit '.port = 8080' config.jsonc
{
  // Server settings
  "host": "localhost",
  "port": 8080, // default port
  /* Feature flags */
  "debug": false
}
```

jq computes the result; jqc applies only the changed values to the original text and prints it, so comments and formatting stay. `--in-place` writes it back to the file instead.

## Install

**Homebrew** (also installs jq)

```bash
brew install muleyuck/tap/jqc
```

**Cargo**: install [jq](https://jqlang.org/download/) separately.

```bash
cargo install --git https://github.com/muleyuck/jqc
```

Prebuilt archives for macOS and Linux are attached to each [release](https://github.com/muleyuck/jqc/releases). The shell installer (`jqc-installer.sh`) was last published with v0.3.0.

Filters, `--edit` and `--in-place` need jq; `fmt` does not.

## Usage

### Read

```console
$ jqc '.port' config.jsonc
3000
$ jqc -r '.host' config.jsonc
localhost
```

Filters and options are jq's: see the [jq manual](https://jqlang.org/manual/). [More](docs/reference.md#filter)

### Format and validate

```console
$ jqc fmt config.jsonc > /dev/null && echo valid
valid
$ echo '{"a": }' | jqc fmt
Error: Failed to parse JSONC: Unexpected close brace on line 1 column 7
```

`fmt` prints the file with its comments (colored on a terminal) and exits with 5 on invalid JSONC (2 if the file can't be read), so it works as a pre-commit check. It doesn't need jq. [More](docs/reference.md#fmt)

### Edit in place

```console
$ jqc --in-place '.debug = true' config.jsonc
$ jqc '.debug' config.jsonc
true
```

`--edit` prints the edited file; `--in-place` writes it back to each file. [How edits are written back](docs/reference.md#edit)

## Reference

[docs/reference.md](docs/reference.md) covers the details:

- how input is converted, and jqc's own options
- `fmt`
- how edits are written back, and the jq options an edit can't use
- colors and `JQ_COLORS`
- exit statuses
- differences from jq
- development

## License

MIT
