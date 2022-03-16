# 🍐 elm-pair

An artificial pair-programmer that helps you writing Elm code.

[Check out this 2 minute demo of current functionality!][demo]

The current version of this code is licensed under GPL. The plan is for a change of license at some point in the future, after which you will need a payed license to be able to use elm-pair commercially.

## Installation

Currently elm-pair only has support for [Neovim][] running on Linux or MacOS.

### Using a Neovim plugin manager

There's a lot of Neovim plugin managers, too many to list them all here! You'll want to add the `neovim-plugin/` subdirectory of this repository as a plugin to your Neovim configuration. If you're running into trouble please create an issue on this repository, I'm happy to help!

The Neovim plugin will perform some installation steps the first time you open a `.elm` file in Neovim with this plugin enabled.

### Using nix home-manager

If you're managing your Neovim configuration using [home-manager][] then you can add elm-pair to your list of plugins. You won't need to install the elm-pair program separately.

```nix
{ pkgs, ... }:

{
  programs.neovim = {
    enable = true;
    plugins =
      let
        elm-pair = pkgs.fetchFromGitHub {
          owner = "jwoudenberg";
          repo = "elm-pair";
          rev = "main";
          sha256 = lib.fakeSha256;
        };
      in [ (import elm-pair).neovim-plugin ];
  };
}
```

Building your environment for the first time will fail with a hash mismatch error. Replace `lib.fakeSha256` in the code above with the correct hash provided in the error message, run again, and you should be all set.

## Acknowledgements

This project is made possible by a couple of others.

- [tree-sitter][] is a library for fast code parsing. It allows Elm-pair to listen to every key stroke and figure out programmer intent quickly and efficiently.
- [tree-sitter-elm][] is an extension for tree-sitter that adds support for the Elm programming language.
- [ropey][] provides the 'rope' datastructure Elm-pair uses to store local copies of source code.
- [notify][] makes it easy for Elm-pair to subscribe to changes in the file system, so it can keep up with what's happening in your Elm projects.
- [differential-dataflow][] provides a way to do incremental computation, allowing Elm-pair to do the bare minimum of work when a file changes.

[demo]: https://vimeo.com/662666351
[differential-dataflow]: https://crates.io/crates/differential-dataflow
[home-manager]: https://github.com/nix-community/home-manager
[neovim]: https://neovim.io/
[notify]: https://crates.io/crates/notify
[ropey]: https://crates.io/crates/ropey
[tree-sitter-elm]: https://github.com/elm-tooling/tree-sitter-elm
[tree-sitter]: https://tree-sitter.github.io/tree-sitter/
