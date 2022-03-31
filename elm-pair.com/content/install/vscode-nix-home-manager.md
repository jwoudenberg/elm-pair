+++
title = "Installing Elm-pair for Visual Studio Code using Nix home-manager"
template = "page.html"
+++

This page describes how to install Elm-pair for Visual Studio Code using [home-manager][]. It assumes you're already using home-manager for managing your Visual Studio Code configuration.

If you run into trouble we'd love to help. Please [reach out](/support)!

1. Open your home-mananger configuration and modify the section configuring Visual Studio Code to make it look like this:

   ```nix
   { pkgs, ... }:

   {
     programs.vscode = {
       enable = true;
       extensions =
         let
           elm-pair = pkgs.fetchFromGitHub {
             owner = "jwoudenberg";
             repo = "elm-pair";
             rev = "release-latest";
             sha256 = lib.fakeSha256;
           };
         in [ (import elm-pair).vscode-extension ];
     };
   }
   ```

1. Save your home-manager configuration file and apply it by running `home-manager switch`. This command will fail with a hash mismatch error. Replace `lib.fakeSha256` in the code above with the correct hash provided in the error message.

1. run `home-manager switch` again to install Elm-pair.

[home-manager]: https://github.com/nix-community/home-manager
