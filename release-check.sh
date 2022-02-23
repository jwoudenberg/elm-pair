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
grep "^\s*\"version\": \"0.$release.0\",$" < vscode-extension/package.json

# check vscode-extension/package-lock.json contains the expected version
grep "^\s*\"version\": \"0.$release.0\",$" < vscode-extension/package-lock.json

# check changelog contains an entry for this version
grep -P "## \d{4}-\d{2}-\d{2}: Release $release$" < CHANGELOG.md

# check news entry on elm-pair.jasperwoudenberg.com
grep -P "<h3>\d{4}-\d{2}-\d{2}: Release $release</h3>" < elm-pair.jasperwoudenberg.com/index.html

# check RSS feed entry on elm-pair.jasperwoudenberg.com
grep -P "<title>Release $release</title>" < elm-pair.jasperwoudenberg.com/feed.xml

# check version downloaded by neovim plugin install script
grep "^ELM_PAIR_VERSION=\"release-$release\"$" < neovim-plugin/elm-pair

# Check cargo build runs without producing warnings
(cd elm-pair && RUSTFLAGS=-Dwarnings cargo build --release)

# Check nix-build runs (runs tests too)
nix build

# check repository does not contain uncomitted changes
if git status --porcelain | grep . ; then
  fail "Stash any changes before starting the release script."
fi

# check github release tag exists
git fetch --tags
git tag -l --points-at HEAD | grep "^release-$release$"
