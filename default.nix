let
  flake-compat =
    builtins.fetchGit { url = "https://github.com/edolstra/flake-compat.git"; };
  flake = (import flake-compat { src = ./.; }).defaultNix;
in flake.packages.${builtins.currentSystem}
