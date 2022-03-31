#!/usr/bin/env bash

set -euxo pipefail

# Script that checks all necessary steps for a release have been taken.

function fail {
  echo "$1"
  exit 1
}

release="$1"

# check elm-pair/Cargo.toml contains the expected version
grep "^version = \"0.$release.0\"$" < elm-pair/Cargo.toml

# check elm-pair/Cargo.lock contains the expected version
grep -zlP "name = \"elm-pair\"\nversion = \"0.$release.0\"" < elm-pair/Cargo.toml

# check vscode-extension/package.json contains the expected version
grep "^\s*\"version\": \"0.$release.0\",$" < editor-integrations/vscode/package.json

# check changelog contains an entry for this version
grep -P "## \d{4}-\d{2}-\d{2}: Release $release$" < CHANGELOG.md

# check version downloaded by neovim plugin install script
grep "^ELM_PAIR_VERSION=\"release-$release\"$" < editor-integrations/neovim/elm-pair

# Check cargo build runs without producing warnings
(cd elm-pair && cargo build --release)

# Check nix-build runs (runs tests too)
nix build

# check repository does not contain uncomitted changes
if git status --porcelain | grep . ; then
  fail "Stash any changes before starting the release script."
fi

# check github release tag exists
git fetch --tags
git tag -l --points-at HEAD | grep "^release-$release$"

# Check `release-latest` branch points at latest release
git branch -l --points-at HEAD | grep '^release-latest$'
