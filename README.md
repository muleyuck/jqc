[![unit-test](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml/badge.svg)](https://github.com/muleyuck/jqc/actions/workflows/unit-test.yml)
![Software License](https://img.shields.io/badge/license-MIT-brightgreen.svg?style=flat-square)
[![Release](https://img.shields.io/github/release/muleyuck/jqc.svg)](https://github.com/muleyuck/jqc/releases/latest)

# jqc

**🧩 jq for JSONC — query, view, and edit JSON-with-Comments files without losing your comments.**

`jq` is the standard tool for JSON on the command line. But many config files — VS Code `settings.json`, `tsconfig.json`, `deno.jsonc`, `biome.jsonc` — use JSONC, which extends JSON with `//` and `/* */` comments. Piping these through `jq` silently strips every comment.

![demo](https://github.com/user-attachments/assets/24711d01-76b0-4a37-a3ed-e13a90a62696)

## Install

**Homebrew**

```bash
brew install muleyuck/tap/jqc
```

**Shell script (macOS / Linux)**

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/muleyuck/jqc/releases/latest/download/jqc-installer.sh | sh
```

**Cargo**

```bash
cargo install --git https://github.com/muleyuck/jqc
```

---

## 1. Query with jq syntax

`jqc` accepts the same filter expressions as `jq`. If you already know `jq`, you already know how to query with `jqc`.

```bash
jqc '.port' config.jsonc
# 3000

jqc '.compilerOptions.target' tsconfig.json
# "ES2022"

jqc '.plugins[]' config.jsonc
# "core"
# "auth"

jqc '.plugins[0]' config.jsonc
# "core"

cat config.jsonc | jqc '.host'
# "localhost"
```

**Output flags** (same as `jq`)

| Flag | Behavior |
|------|----------|
| `-r` | Raw output — strips surrounding quotes from strings |
| `-c` | Compact output — no newlines |

> `jqc` uses [jaq](https://github.com/01mf02/jaq) as its filter engine. The vast majority of `jq` filters work without modification. For known differences, see the [jaq compatibility notes](https://github.com/01mf02/jaq?tab=readme-ov-file#differences-from-jq).

---

## 2. View JSONC with color and comments

Running `jqc` without a filter, or using `fmt`, outputs JSONC with syntax highlighting. Comments are colorized alongside the JSON tokens — something `jq` cannot do because it cannot parse JSONC at all.

```bash
# Colorized output — comments are preserved
jqc fmt config.jsonc

# Identity filter — pretty-prints with color, but comments are stripped
# (the filter engine processes pure JSON values and does not carry comments through)
jqc '.' config.jsonc
```

Output when writing to a terminal is colorized automatically. When piped, output is plain. Override with:

```bash
jqc -C fmt config.jsonc         # force color (e.g. when piping to less -R)
jqc -M fmt config.jsonc         # disable color
NO_COLOR=1 jqc fmt config.jsonc # disable color (https://no-color.org/)
```

Token colors are customizable via `JQC_COLORS` — a colon-separated list of 9 ANSI SGR codes:

```
null : false : true : number : string : array : object : key : comment
```

Leave a field empty to keep the default. Example — bold cyan numbers:

```bash
export JQC_COLORS="::::1;36::::"
jqc fmt config.jsonc
```

`fmt` also validates JSONC syntax and exits non-zero on invalid input, making it usable as a pre-commit check:

```bash
jqc fmt tsconfig.json > /dev/null && echo "valid"
```

---

## 3. Edit while preserving comments

`set`, `del`, and `push` rewrite the JSONC source text directly. Only the targeted value changes — all comments, including inline comments on the same line as the edited value, are left untouched.

```
Before                              After: jqc set '.port' 8080 -i config.jsonc
──────────────────────────────────  ──────────────────────────────────────────────
{                                   {
  // Server settings                  // Server settings
  "host": "localhost",                "host": "localhost",
  "port": 3000, // default port  →    "port": 8080, // default port
  /* Feature flags */                 /* Feature flags */
  "debug": false                      "debug": false
}                                   }
```

Without `-i`, the result is printed to stdout. Add `-i` to overwrite the file atomically.

### `set` — update a value

```bash
jqc set '.port' 8080 config.jsonc                       # print to stdout
jqc set '.port' 8080 -i config.jsonc                    # edit in-place

jqc set '.host' '"production.example.com"' config.jsonc # string value
jqc set '.compilerOptions.strict' 'false' tsconfig.json # boolean
jqc set '.compilerOptions.target' '"ES2022"' tsconfig.json
```

### `del` — remove a key

```bash
jqc del '.debug' config.jsonc
jqc del '.compilerOptions.noImplicitAny' -i tsconfig.json
```

### `push` — append to an array

```bash
jqc push '.plugins' '"logging"' config.jsonc
jqc push '.lint.rules.tags' '"strict"' -i deno.jsonc
```

---

## Comparison with jq

| | jq | jqc |
|---|---|---|
| Filter syntax | yes | yes — same syntax |
| Reads JSONC | no — parse error | yes |
| Colorized output with comments | no | yes |
| Comment-preserving edits | no | yes |
| In-place editing | no | yes (`-i`) |
| Custom colors | `JQ_COLORS` (8 fields) | `JQC_COLORS` (9 fields, adds comment color) |

## License

MIT
