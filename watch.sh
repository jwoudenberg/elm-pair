#!/usr/bin/env nix-shell
#! nix-shell -i bash -p entr

# Watch source files and recompile when any change.

git ls-files | grep .rs | entr -c -s "cargo clippy --tests && cargo test $1"
