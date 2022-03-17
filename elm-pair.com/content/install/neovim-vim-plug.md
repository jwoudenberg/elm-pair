+++
title = "Installing Elm-pair for Neovim using vim-plug"
template = "page.html"
+++

This page describes how to install Elm-pair for Neovim using the [vim-plug][] plugin manager. It assumes vim-plug has already been installed.

If you run into trouble we'd love to help. Please [reach out](/support)!

1. Open your Neovim configuration file. It is typically located either at `~/.vimrc` or `~/.config/neovim/init.vim`.

1. Find the section of the configuration file listing plugins, or create this section if it does not exist. Then add the Elm-pair plugin, so the end result looks like this:

   ```vimscript
   call plug#begin()
   Plug 'jwoudenberg/elm-pair', { 'rtp': 'editor-integrations/neovim' }

   " .. potentially more plugins here!

   call plug#end()
   ```

1. Save the configuration file and start a fresh Neovim. In it type `:PlugInstall` and hit enter to install Elm-pair.

[vim-plug]: https://github.com/junegunn/vim-plug
