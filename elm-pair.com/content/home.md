+++
title = "Home"
template = "page.html"
+++

Elm-pair helps you write Elm code. You tell Elm-pair about the change you want to make and it will do the actual work. It's a bit like using an IDE, except you don't need to learn any keyboard shortcuts.

You talk to Elm-pair by making a change in your code. If Elm-pair understands your intent then it will follow up with its own change.

### Renaming variables, types, and constructors

Rename a variable and Elm-pair will propagate the new name wherever the variable is used. Elm-pair will propagate type and constructor names in the same way.

![Demonstration of rename functionality in Visual Studio Code](/renaming.gif)

### Changing import statements

As you change an import statement alias or exposing list, Elm-pair will update your code to keep it compiling.

![Demonstration of import statement functionality in Visual Studio Code](/imports.gif)

### Use your own editor

Elm-pair integrates with your editor of choice. Currently Visual Studio Code and Neovim are supported, with support for additional editors planned. Elm-pair runs on MacOS and Linux.
