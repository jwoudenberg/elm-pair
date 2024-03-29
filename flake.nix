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
    let supportedSystems = [ "x86_64-linux" "x86_64-darwin" "aarch64-darwin" ];
    in utils.lib.eachSystem supportedSystems (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        system-fenix = fenix.packages.${system};
        rust-toolchain = system-fenix.combine [
          system-fenix.stable.rustc
          system-fenix.stable.cargo
          system-fenix.stable.rustfmt
          system-fenix.stable.clippy
          system-fenix.targets."x86_64-unknown-linux-musl".stable.rust-std
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

        site = pkgs.stdenv.mkDerivation {
          src = ./.;
          name = "elm-pair.com";
          buildPhase = ''
            cd elm-pair.com
            ${pkgs.python38}/bin/python ./changelog-to-news.py
            ${pkgs.zola}/bin/zola build
          '';
          installPhase = ''
            mv public $out
          '';
        };

        elm-pair = system-naersk.buildPackage {
          pname = "elm-pair";
          root = ./elm-pair;
          doCheck = true;
          ELM_BINARY_PATH = "${pkgs.elmPackages.elm}/bin/elm";
        };
        elm-pair-app = utils.lib.mkApp { drv = elm-pair; };

        neovim-plugin = pkgs.vimUtils.buildVimPlugin {
          name = "elm-pair";
          src = ./editor-integrations/neovim;
          preFixup = ''
            substituteInPlace "$out/lua/elm-pair.lua" \
              --replace '"elm-pair"' '"${elm-pair}/bin/elm-pair"'
          '';
        };

        vscode-extension = pkgs.vscode-utils.buildVscodeExtension {
          name = "elm-pair";
          src = ./editor-integrations/vscode;
          vscodeExtUniqueId = "jwoudenberg.elm-pair";
          preBuild = ''
            cp ${./README.md} ./README.md
            cp ${./CHANGELOG.md} ./CHANGELOG.md
            cp ${./LICENSE} ./LICENSE
            cp ${./elm-pair.com/static/logo.png} ./icon.png
            cp ${./editor-integrations/neovim/elm-pair} ./elm-pair
            substituteInPlace "extension.js" \
              --replace 'nix-build-put-path-to-elm-pair-here' '${elm-pair}/bin/elm-pair'
          '';
        };
      in {
        # Packages
        defaultPackage = elm-pair;
        packages.site = site;
        packages.elm-pair = elm-pair;
        packages.neovim-plugin = neovim-plugin;
        packages.vscode-extension = vscode-extension;

        # Apps
        defaultApp = elm-pair-app;
        apps.elm-pair = elm-pair-app;

        # Checks
        checks.vscode-extension = pkgs.runCommand "vscode-extension" { } ''
          ${pkgs.nodejs}/bin/node ${
            ./editor-integrations/vscode
          }/tests.js > $out
        '';

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
            pkgs.nodePackages.prettier

            # Site development
            pkgs.zola
            pkgs.python38

            # Licensing server
            pkgs.go
          ];
        };
      });
}
