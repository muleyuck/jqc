#!/usr/bin/env bash
# Runs every case in tests/fixtures/jq-compat/cases.json through both jq and jqc
# (as found in PATH) and checks each output against the case's expectation.
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
  # Lines are joined with newlines, so a line holding one would alias another expectation.
  def lines: type == "array" and all(type == "string" and (contains("\n") | not));
  # jq keeps number literals as written, so 5.0 would be compared as the text "5.0".
  def integer($pattern): type == "number" and (tostring | test($pattern));
  def exit_status: integer("^[0-9]+$");
  (.cases | to_entries[] | .key as $i | .value as $c | "case \($i) in cases.json: " as $at
   | if ($c | type) != "object" then $at + "must be an object" else
     (($c | keys - ["name", "args", "stdin", "files", "expect", "status", "jqc", "jqc_status", "known_difference", "note"])[]
      | $at + "unknown field \(tojson)"),
     (if ($c.name | type) != "string" then $at + "\"name\" must be a string" else empty end),
     (if ($c.args | type) != "array" or ($c.args | length) == 0 or ($c.args | any(type != "string"))
      then $at + "\"args\" must be a non-empty array of strings" else empty end),
     (if ($c.stdin | type) != "string" and $c.stdin != null then $at + "\"stdin\" must be a string" else empty end),
     (if $c.files != null and (($c.files | type) != "object" or ($c.files | any(type != "string"))
        or ($c.files | keys | any(. == "" or . == "." or . == ".." or contains("/"))))
      then $at + "\"files\" must map plain file names to strings" else empty end),
     (if $c.expect | lines then empty else $at + "\"expect\" must be an array of lines, each without a newline" end),
     (if $c.jqc == null or ($c.jqc | lines) then empty else $at + "\"jqc\" must be an array of lines, each without a newline" end),
     ((["status", "jqc_status"][]) as $f
      | if $c[$f] == null or ($c[$f] | exit_status) then empty else $at + "\"\($f)\" must be an exit status" end),
     (if $c.known_difference != null and ($c.known_difference | integer("^[1-9][0-9]*$") | not)
      then $at + "\"known_difference\" must be an issue number" else empty end),
     (if $c.known_difference != null and $c.note != null
      then $at + "use either \"known_difference\" or \"note\", not both" else empty end),
     (if $c.note != null and (($c.note | type) != "string" or $c.note == "") then $at + "\"note\" must be a non-empty string" else empty end),
     (($c | has("jqc") or has("jqc_status")) as $differs
      | if $differs and $c.known_difference == null and $c.note == null
        then $at + "a jqc expectation needs a \"known_difference\" or a \"note\" explaining it"
        elif ($differs | not) and ($c.known_difference != null or $c.note != null)
        then $at + "\"known_difference\" and \"note\" only apply when jqc has its own expectation"
        elif $differs and ($c.jqc // $c.expect) == $c.expect and ($c.jqc_status // $c.status // 0) == ($c.status // 0)
        then $at + "the jqc expectation is the same as \"expect\"; remove it"
        else empty end)
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

# Writes the expected stdout and exit status of one tool; the lines each end in a newline.
expectation() {
  local dir="$work/$1" lines=$2 status=$3
  mkdir -p "$dir"
  jq -j "$lines | map(. + \"\\n\") | join(\"\")" "$work/case.json" > "$dir/stdout"
  jq -r "$status" "$work/case.json" > "$dir/status"
}

matches() {
  cmp -s "$work/$1/stdout" "$work/$2/stdout" && cmp -s "$work/$1/status" "$work/$2/status"
}

describe() {
  local actual=$1 expected=$2
  echo "  args:     $(jq -c '.args' "$work/case.json")"
  printf '  expected: status %s, stdout %s\n' "$(cat "$work/$expected/status")" "$(jq -Rs . "$work/$expected/stdout")"
  printf '  actual:   status %s, stdout %s\n' "$(cat "$work/$actual/status")" "$(jq -Rs . "$work/$actual/stdout")"
}

count=$(jq '.cases | length' "$cases")
same=0 known=0 intended=0 failures=0
i=0
while [ "$i" -lt "$count" ]; do
  jq -c --argjson i "$i" '.cases[$i]' "$cases" > "$work/case.json"
  name=$(jq -r '.name' "$work/case.json")
  issue=$(jq -r '.known_difference // empty' "$work/case.json")
  differs=$(jq -r 'has("jqc") or has("jqc_status")' "$work/case.json")
  args=()
  while IFS= read -r -d '' arg; do
    args+=("$arg")
  done < <(jq -j '.args[] | . + "\u0000"' "$work/case.json")
  jq -j '.stdin // ""' "$work/case.json" > "$work/stdin"

  run jq
  run jqc
  expectation jq-expected '.expect' '.status // 0'
  expectation jqc-expected '.jqc // .expect' '.jqc_status // .status // 0'

  if ! matches jq jq-expected; then
    echo "FAIL $name: jq's output doesn't match \"expect\""
    describe jq jq-expected
    failures=$((failures + 1))
  fi
  if matches jqc jqc-expected; then
    if [ "$differs" = false ]; then
      same=$((same + 1))
    elif [ -n "$issue" ]; then
      known=$((known + 1))
    else
      intended=$((intended + 1))
    fi
  elif [ "$differs" = true ] && matches jqc jq-expected; then
    if [ -n "$issue" ]; then
      echo "FAIL $name: jqc now prints what \"expect\" says, so #$issue looks fixed: remove its jqc expectation and known_difference"
    else
      echo "FAIL $name: jqc now prints what \"expect\" says: remove its jqc expectation and note"
    fi
    failures=$((failures + 1))
  else
    echo "FAIL $name: jqc's output doesn't match its expectation"
    describe jqc jqc-expected
    failures=$((failures + 1))
  fi
  i=$((i + 1))
done

echo "$count cases, $failures failures: $same same as jq, $known known differences, $intended intended differences ($(jq --version), jqc at $(command -v jqc))"
[ "$failures" -eq 0 ]
