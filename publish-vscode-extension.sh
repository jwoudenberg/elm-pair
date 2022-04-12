#!/usr/bin/env nix-shell
#! nix-shell -i bash -p nodejs
# shellcheck shell=bash

set -euxo pipefail

# Script that publishes a new version of the visual studio code extension.

TMPDIR=$(mktemp -d)

# Build the extension using Nix.
nix build .#vscode-extension
shopt -s dotglob # include hidden files in the copy wildcard below.
cp -r result/share/vscode/extensions/jwoudenberg.elm-pair/* "$TMPDIR/"
chmod -R 700 "$TMPDIR"
pushd "$TMPDIR"

# Nix build changed the '!# /usr/bin/env bash' schebang with a nix path.
# We change it back to support more operating systems.
grep /nix/store elm-pair # fail this script if our assumptions no longer hold
sed -i "s|/nix/store/.*/bin/bash|/usr/bin/env bash|g" elm-pair
if grep /nix/store elm-pair; then
  echo "Nix paths remain in elm-pair installation script"
  exit 1
fi

# Publish it!
npx vsce login hjgames
npx vsce publish
