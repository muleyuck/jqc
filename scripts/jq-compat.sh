#!/usr/bin/env bash
# Regenerates tests/fixtures/jq-compat/expected.json by running every case in
# cases.json through the jq in PATH.
set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq not found in PATH" >&2
  exit 1
fi

root=$(cd "$(dirname "$0")/.." && pwd)
fixtures="$root/tests/fixtures/jq-compat"
cases="$fixtures/cases.json"
expected="$fixtures/expected.json"

version=$(jq --version)
version=${version#jq-}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
: > "$work/entries"

count=$(jq '.cases | length' "$cases")
i=0
while [ "$i" -lt "$count" ]; do
  jq -c --argjson i "$i" '.cases[$i]' "$cases" > "$work/case.json"
  if ! jq -e '.args | type == "array" and length > 0 and all(type == "string")' "$work/case.json" >/dev/null; then
    echo "error: case $i in cases.json: \"args\" must be a non-empty array of strings" >&2
    exit 1
  fi
  run="$work/run$i"
  mkdir "$run"

  while IFS= read -r -d '' file; do
    jq -j --arg f "$file" '.files[$f]' "$work/case.json" > "$run/$file"
  done < <(jq -j '(.files // {}) | keys[] | . + "\u0000"' "$work/case.json")

  args=()
  while IFS= read -r -d '' arg; do
    args+=("$arg")
  done < <(jq -j '.args[] | . + "\u0000"' "$work/case.json")

  jq -j '.stdin // ""' "$work/case.json" > "$work/stdin"

  status=0
  (cd "$run" && jq "${args[@]}" < "$work/stdin" > "$work/stdout" 2>/dev/null) || status=$?

  jq -n -c --slurpfile c "$work/case.json" --rawfile stdout "$work/stdout" --argjson status "$status" \
    '{($c[0].name): {stdout: $stdout, status: $status}}' >> "$work/entries"
  i=$((i + 1))
done

jq -n --slurpfile entries "$work/entries" '{cases: ($entries | add // {})}' > "$expected"
echo "wrote ${expected#"$root"/} ($count cases, jq $version)"
