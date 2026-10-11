#!/bin/sh
# Packages one built binary as a release archive, named and laid out as
# cargo-dist did: jqc-<target>.tar.xz holding jqc-<target>/{jqc,LICENSE,
# README.md,CHANGELOG.md}, and jqc-<target>.tar.xz.sha256.
# Usage: scripts/package.sh <target> <binary> <out-dir>
set -eu
target=$1
binary=$2
out=$3
name="jqc-$target"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/$name" "$out"
cp "$binary" "$work/$name/jqc"
cp LICENSE README.md CHANGELOG.md "$work/$name/"
tar -C "$work" -cJf "$out/$name.tar.xz" "$name"
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$name.tar.xz" > "$name.tar.xz.sha256"
else
  shasum -a 256 "$name.tar.xz" > "$name.tar.xz.sha256"
fi
