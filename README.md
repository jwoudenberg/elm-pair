# 🍐 elm-pair

An artificial pair-programmer that helps you writing Elm code.

[Check out this 2 minute demo of current functionality!][demo]

The current version of this code is licensed under GPL. The plan is for a change of license at some point in the future, after which you will need a payed license to be able to use elm-pair commercially.

## Installation

Currently elm-pair only has support for [Neovim][] running on Linux or MacOS, and installation using Nix.

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

### Using a Neovim plugin manager

Follow the instructions of your plugin manager to add the plugin located in the `./neovim-plugin` directory of this repository. When installed this way the plugin will not include the elm-pair binary, and you will need to make sure it is available on your `$PATH`.

#### Install as a user package using [home-manage][]

Include elm-pair in your `home-manager.nix`:

```nix
{ pkgs, ... }: {
  home.packages =
    let
      elm-pair = pkgs.fetchFromGitHub {
        owner = "jwoudenberg";
        repo = "elm-pair";
        rev = "main";
        sha256 = lib.fakeSha256;
      };
    in [ (import elm-pair).elm-pair ];
}
```

Building your environment for the first time will fail with a hash mismatch error. Replace `lib.fakeSha256` in the code above with the correct hash provided in the error message, run again, and you should be all set.

#### Install as a system package using [nixos][] or [nix-darwin][]

Include elm-pair in your `configuration.nix`:

```nix
{ pkgs, ... }: {
  environment.systemPackages =
    let
      elm-pair = pkgs.fetchFromGitHub {
        owner = "jwoudenberg";
        repo = "elm-pair";
        rev = "main";
        sha256 = lib.fakeSha256;
      };
    in [ (import elm-pair).elm-pair ];
}
```

Building your environment for the first time will fail with a hash mismatch error. Replace `lib.fakeSha256` in the code above with the correct hash provided in the error message, run again, and you should be all set.

#### Install as a project-dependency using [nix-shell][]

Include elm-pair your project's `shell.nix`. Note: when set up this way the neovim plugin will only work when you start Neovim from the project shell provided by nix-shell.

```nix
{ pkgs, ...  }:
  pkgs.mkShell {
    buildInputs =
      let
        elm-pair = pkgs.fetchFromGitHub {
          owner = "jwoudenberg";
          repo = "elm-pair";
          rev = "main";
          sha256 = lib.fakeSha256;
        };
      in [ (import elm-pair).elm-pair ];
}
```

Building your environment for the first time will fail with a hash mismatch error. Replace `lib.fakeSha256` in the code above with the correct hash provided in the error message, run again, and you should be all set.

[demo]: https://vimeo.com/662666351
[home-manager]: https://github.com/nix-community/home-manager
[neovim]: https://neovim.io/
[nix-darwin]: https://github.com/LnL7/nix-darwin
[nix-shell]: https://nix.dev/tutorials/ad-hoc-developer-environments
[nixos]: https://nixos.org/
