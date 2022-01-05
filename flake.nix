{
  description = "elm-pair";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, utils, naersk }:
    utils.lib.eachDefaultSystem (system:
      let pkgs = nixpkgs.legacyPackages.${system};
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
        packages.elm-pair = naersk.lib."${system}".buildPackage {
          pname = "elm-pair";
          root = ./.;
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
            pkgs.libiconv
            pkgs.luaformatter
            pkgs.lua53Packages.luacheck
            pkgs.cargo
            pkgs.elmPackages.elm
            pkgs.elmPackages.elm-format
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
          ];
          RUST_SRC_PATH =
            "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
        };
      });
}
