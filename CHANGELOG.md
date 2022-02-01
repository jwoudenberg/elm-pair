# Changelog

## 2022-02-01: Release 6

- Fixed a but where Elm-pair would sometimes crash if specific dependencies were included in the `elm.json` file of a project.
- Make Elm-pair less eager to rename. When it's unclear whether the programmer intent is to rename or, for example, call a function with a different name, Elm-pair will do nothing.

## 2022-01-31: Release 5

- Elm-pair supports work on Elm packages.

## 2022-01-31: Release 4

- Fixes elm-pair installation script bundled with Neovim plugin on MacOS.

## 2022-01-31: Release 3

- Support for renaming within a single module. Change the name of any variable, type, or constructor and Elm-pair will update other usages of the name within the same module. No support for renames across multiple modules yet.

## 2022-01-17: Release 2

- Nix is no longer a dependency for installing Elm-pair.
- Elm-pair no longer reparses the entire project when a single Elm module changes.
- Many bugs have been squashed.

## 2022-01-06: Release 1

- First release, with support for managing import statements.
