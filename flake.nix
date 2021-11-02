{
  description = "elm-pair";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-21.05";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = nixpkgs.legacyPackages.${system};
      in {
        devShell = pkgs.mkShell {
          buildInputs = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy ];
          RUST_SRC_PATH =
            "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
        };
      });
}
