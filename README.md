# <img alt="Elm-pair logo" height="45px" src="https://elm-pair.com/logo.png"> Elm-pair

Elm-pair helps you write Elm code. You tell Elm-pair about the change you want to make and it will do the actual work. It's a bit like using an IDE, except you don't need to learn any keyboard shortcuts.

You talk to Elm-pair by making a change in your code. If Elm-pair understands your intent then it will follow up with its own change.

### Renaming variables, types, and constructors

Rename a variable and Elm-pair will propagate the new name wherever the variable is used. Elm-pair will propagate type and constructor names in the same way.

![Demonstration of rename functionality in Visual Studio Code][renaming-gif]

### Changing import statements

As you change an import statement alias or exposing list, Elm-pair will update your code to keep it compiling.

![Demonstration of import statement functionality in Visual Studio Code][imports-gif]

### Use your own editor

Elm-pair integrates with your editor of choice. Currently Neovim is supported and Visual Studio Code support is on the way, with support for additional editors planned. Elm-pair runs on MacOS and Linux.

## Get it

You can find installation instructions at https://elm-pair.com/install.

## Acknowledgements

This project is made possible by a couple of others.

- [tree-sitter][] is a library for fast code parsing. It allows Elm-pair to listen to every key stroke and figure out programmer intent quickly and efficiently.
- [tree-sitter-elm][] is an extension for tree-sitter that adds support for the Elm programming language.
- [ropey][] provides the 'rope' datastructure Elm-pair uses to store local copies of source code.
- [notify][] makes it easy for Elm-pair to subscribe to changes in the file system, so it can keep up with what's happening in your Elm projects.
- [differential-dataflow][] provides a way to do incremental computation, allowing Elm-pair to do the bare minimum of work when a file changes.

[differential-dataflow]: https://crates.io/crates/differential-dataflow
[home-manager]: https://github.com/nix-community/home-manager
[imports-gif]: https://elm-pair.com/imports.gif
[neovim]: https://neovim.io/
[notify]: https://crates.io/crates/notify
[renaming-gif]: https://elm-pair.com/renaming.gif
[ropey]: https://crates.io/crates/ropey
[tree-sitter-elm]: https://github.com/elm-tooling/tree-sitter-elm
[tree-sitter]: https://tree-sitter.github.io/tree-sitter/
