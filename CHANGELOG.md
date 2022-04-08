# Changelog

## 2022-04-08: Release 16

- Fixed a bug where renaming an argument would sometimes cause Elm-pair to needlessly add a suffix to the name of arguments of other, unrelated functions.

## 2022-04-07: Release 15

- When changing a name Elm-pair will now change usages of that name in other files as well.
- Fixed a bug in the Visual Studio Code extension, where the first change made to an Elm file would never result in a refactor by Elm-pair.
- Fixed a bug where Elm-pair would remove the wrong qualifier from a name.

## 2022-03-22: Release 14

- Linux binaries are now statically linked. This will let them run on some previously unsupported linux systems.

## 2022-03-17: Release 13

- Add support for Visual Studio Code.
- From this version forward, using Elm-pair for commercial work requires a payed license. This change is intended to fund further development of Elm-pair.

## 2022-03-14: Release 12

- Opening an Elm module outside an Elm project directory no longer crashes Elm-pair.

## 2022-03-04: Release 11

- When giving a variable a new name that is already in use in a separate
scope, Elm-pair will no longer add a suffix to that other variable, moving it
out of the way.

## 2022-02-09: Release 10

- Fixes a bug where typing fast would cause Elm-pair to loose the thread on a rename refactor.

## 2022-02-03: Release 9

- Fixes a bug where undo-ing a refactor made by Elm-pair would result in a new refactor, preventing the programmer from moving further back into their undo history.

## 2022-02-03: Release 8

- Elm-pair no longer starts a rename refactor when the programmer changes the name of a constructor at a usage site, as opposed to where the constructor is defined.

## 2022-02-02: Release 7

- Elm-pair no longer generates index.html files when running `elm-make`.
- Elm-pair now responds with a rename refactor when the programmer changes the name where it is defined, no longer when the programmer changes a usage of the name. The old behavior sometimes misinterpreted programmer intent.

## 2022-02-01: Release 6

- Fixed a bug where Elm-pair would sometimes crash if specific dependencies were included in the `elm.json` file of a project.
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
