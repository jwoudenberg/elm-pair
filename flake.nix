{
  description = "elm-pair";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
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
          cargo = rust-toolchain;
          rustc = rust-toolchain;
        };
      in rec {
        # Packages
        packages.neovim-plugin = pkgs.vimUtils.buildVimPlugin {
          name = "elm-pair";
          src = ./neovim-plugin;
          preFixup = ''
            substituteInPlace "$out/lua/elm-pair.lua" \
              --replace '"elm-pair"' '"${packages.elm-pair}/bin/elm-pair"'
          '';
        };
        packages.elm-pair = system-naersk.buildPackage {
          pname = "elm-pair";
          root = ./elm-pair;
          doCheck = true;
          ELM_BINARY_PATH = "${pkgs.elmPackages.elm}/bin/elm";
        };
        defaultPackage = packages.elm-pair;

        # Apps
        apps.elm-pair = utils.lib.mkApp { drv = packages.elm-pair; };
        defaultApp = apps.elm-pair;

        # Development
        devShell = pkgs.mkShell {
          buildInputs = [
            rust-toolchain
            pkgs.nodejs
            pkgs.luaformatter
            pkgs.lua53Packages.luacheck
            pkgs.elmPackages.elm
            pkgs.elmPackages.elm-format
          ];
        };
      });
}
