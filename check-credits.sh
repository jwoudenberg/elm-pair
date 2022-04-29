#!/usr/bin/env nix-shell
#! nix-shell -i bash -p cargo-license difftastic
# shellcheck shell=bash

set -euo pipefail

# Script that checks elm-pair/src/credits.txt is not missing any dependencies

pushd elm-pair/
PACKAGE_PATTERN="[a-zA-Z0-9_\-]+"
USED_DEPS=$(
  cargo-license --avoid-build-deps --do-not-bundle \
    | grep --invert-match --ignore-case --extended-regexp "unlicense|cc0-1.0|bsl-1.0" \
    | grep --extended-regexp --only-matching "^$PACKAGE_PATTERN:" \
    | grep --invert-match "^elm-pair" \
    | sort \
    | uniq
)
CREDITED_DEPS=$(
  grep --extended-regexp --only-matching "^$PACKAGE_PATTERN:" src/credits.txt \
    | sort
)
difftastic <(echo "$USED_DEPS") <(echo "$CREDITED_DEPS")

if [ "$USED_DEPS" != "$CREDITED_DEPS" ]; then
  exit 1;
fi
