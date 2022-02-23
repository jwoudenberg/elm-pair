{
  description = "elm-pair";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    naersk.url = "github:yusdacra/naersk/feat/cargolock-git-deps";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, utils, naersk, fenix }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        system-fenix = fenix.packages.${system};
        rust-toolchain = system-fenix.combine [
          system-fenix.stable.rustc
          system-fenix.stable.cargo
          system-fenix.stable.rustfmt
          system-fenix.stable.clippy
          # Extra dependencies for `cargo build` on darwin.
          (pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
            pkgs.darwin.apple_sdk.frameworks.CoreServices
          ])
        ];
        system-naersk = naersk.lib."${system}".override {
          # The fork of naersk we're on doesn't contain a fix using stable
          # cargo. Untill we move off the fork we need this commented out.
          # cargo = rust-toolchain;
          rustc = rust-toolchain;
        };
        elm-pair = system-naersk.buildPackage {
          pname = "elm-pair";
          root = ./elm-pair;
          doCheck = true;
          ELM_BINARY_PATH = "${pkgs.elmPackages.elm}/bin/elm";
        };

        neovim-plugin = pkgs.vimUtils.buildVimPlugin {
          name = "elm-pair";
          src = ./neovim-plugin;
          preFixup = ''
            substituteInPlace "$out/lua/elm-pair.lua" \
              --replace '"elm-pair"' '"${elm-pair}/bin/elm-pair"'
          '';
        };
        elm-pair-app = utils.lib.mkApp { drv = elm-pair; };

      in {
        # Packages
        defaultPackage = elm-pair;
        packages.elm-pair = elm-pair;
        packages.neovim-plugin = neovim-plugin;

        # Apps
        apps.elm-pair = elm-pair-app;
        defaultApp = elm-pair-app;

        # Development
        devShell = pkgs.mkShell {
          buildInputs = [
            rust-toolchain
            pkgs.elmPackages.elm
            pkgs.elmPackages.elm-format

            # For neovim plugin development
            pkgs.luaformatter
            pkgs.lua53Packages.luacheck

            # For VSCode plugin development
            pkgs.nodejs
            pkgs.nodePackages.typescript
          ];
        };
      });
}
