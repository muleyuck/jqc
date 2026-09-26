#!/usr/bin/env bash
# Runs every case in tests/fixtures/jq-compat/cases.json through both jq and jqc
# (as found in PATH) and reports where jqc's output differs from jq's.
set -euo pipefail

for tool in jq jqc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: $tool not found in PATH" >&2
    exit 1
  fi
done

root=$(cd "$(dirname "$0")/.." && pwd)
cases="$root/tests/fixtures/jq-compat/cases.json"

# An empty or malformed file would otherwise run zero cases and report success.
if ! jq -e -s 'length == 1 and (.[0] | type == "object" and (.cases | type == "array" and length > 0))' \
  "$cases" >/dev/null 2>&1; then
  echo "error: cases.json must be one JSON object whose \"cases\" is a non-empty array" >&2
  exit 1
fi

problems=$(jq -r '
  (.cases | to_entries[] | .key as $i | .value as $c | "case \($i) in cases.json: " as $at
   | if ($c | type) != "object" then $at + "must be an object" else
     (if ($c.name | type) != "string" then $at + "\"name\" must be a string" else empty end),
     (if ($c.args | type) != "array" or ($c.args | length) == 0 or ($c.args | any(type != "string"))
      then $at + "\"args\" must be a non-empty array of strings" else empty end),
     (if ($c.stdin | type) != "string" and $c.stdin != null then $at + "\"stdin\" must be a string" else empty end),
     (if $c.files != null and (($c.files | type) != "object" or ($c.files | any(type != "string"))
        or ($c.files | keys | any(. == "" or . == "." or . == ".." or contains("/"))))
      then $at + "\"files\" must map plain file names to strings" else empty end),
     (if $c.compare == null or $c.compare == "text" or $c.compare == "value" then empty
      else $at + "\"compare\" must be \"text\" or \"value\"" end),
     (if $c.known_difference != null and (($c.known_difference | type) != "number" or $c.known_difference < 1)
      then $at + "\"known_difference\" must be an issue number" else empty end)
     end),
  ([.cases[] | objects | .name] | group_by(.) | map(select(length > 1) | .[0])[]
   | "duplicate case name \(tojson) in cases.json")
' "$cases")
if [ -n "$problems" ]; then
  echo "$problems" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

run() {
  local tool=$1 dir="$work/$1"
  rm -rf "$dir"
  mkdir -p "$dir/cwd"
  while IFS= read -r -d '' file; do
    jq -j --arg f "$file" '.files[$f]' "$work/case.json" > "$dir/cwd/$file"
  done < <(jq -j '(.files // {}) | keys[] | . + "\u0000"' "$work/case.json")
  local status=0
  (cd "$dir/cwd" && "$tool" "${args[@]}" < "$work/stdin" > "$dir/stdout" 2>/dev/null) || status=$?
  echo "$status" > "$dir/status"
}

# Value mode lets jq read both outputs and compare them with its own `==`, so formatting,
# number spelling and duplicate keys are judged exactly as jq judges them.
same_result() {
  cmp -s "$work/jq/status" "$work/jqc/status" || return 1
  if [ "$compare" = value ]; then
    jq -n -e --slurpfile a "$work/jq/stdout" --slurpfile b "$work/jqc/stdout" '$a == $b' >/dev/null 2>&1
  else
    cmp -s "$work/jq/stdout" "$work/jqc/stdout"
  fi
}

describe() {
  local tool
  echo "  args: $(jq -c '.args' "$work/case.json")"
  for tool in jq jqc; do
    printf '  %-4s status %s, stdout %s\n' "$tool:" "$(cat "$work/$tool/status")" "$(jq -Rs . "$work/$tool/stdout")"
  done
}

count=$(jq '.cases | length' "$cases")
matched=0 known=0 failures=0
i=0
while [ "$i" -lt "$count" ]; do
  jq -c --argjson i "$i" '.cases[$i]' "$cases" > "$work/case.json"
  name=$(jq -r '.name' "$work/case.json")
  compare=$(jq -r '.compare // "text"' "$work/case.json")
  issue=$(jq -r '.known_difference // empty' "$work/case.json")
  args=()
  while IFS= read -r -d '' arg; do
    args+=("$arg")
  done < <(jq -j '.args[] | . + "\u0000"' "$work/case.json")
  jq -j '.stdin // ""' "$work/case.json" > "$work/stdin"

  run jq
  run jqc
  if same_result; then
    if [ -n "$issue" ]; then
      echo "FAIL $name: now matches jq, so #$issue looks fixed: remove its known_difference"
      failures=$((failures + 1))
    else
      matched=$((matched + 1))
    fi
  elif [ -n "$issue" ]; then
    known=$((known + 1))
  else
    echo "FAIL $name: differs from jq"
    describe
    failures=$((failures + 1))
  fi
  i=$((i + 1))
done

echo "$count cases: $matched match jq, $known known differences, $failures failures ($(jq --version), jqc at $(command -v jqc))"
[ "$failures" -eq 0 ]
