+++
title = "Installing Elm-pair for Neovim using packer.nvim"
template = "page.html"
+++

This page describes how to install Elm-pair for Neovim using the [packer.nvim][] plugin manager. It assumes packer.nvim has already been installed.

If you run into trouble we'd love to help. Please [reach out](/support)!

1. Open your packer plugin specification file at `~/.config/nvim/lua/plugins.lua`.

1. Add the Elm-pair to the list of plugins, so the end result looks like this:

   ```vimscript
   return require('packer').startup(function()
     use {'jwoudenberg/elm-pair', rtp = 'editor-integrations/neovim'}

     -- .. potentially more plugins here!

   end)
   ```

1. Save the configuration file and start a fresh Neovim. In it type `:PackerCompile` and hit enter to install Elm-pair.

[packer.nvim]: https://github.com/wbthomason/packer.nvim
