if exists('g:loaded_elm_pair')
  finish
endif
let g:loaded_elm_pair = 1

call luaeval('require("elm-pair").start()')
