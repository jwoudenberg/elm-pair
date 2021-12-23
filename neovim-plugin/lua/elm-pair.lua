-- luacheck: read globals vim
function _G.elm_pair_on_elm_buffer()
    local buffer = vim.fn.bufnr("%")
    local path = vim.fn.expand("%:p")
    vim.fn.rpcnotify(_G.elm_pair_channel, "buffer_opened", buffer, path)
end

local function start()
    _G.elm_pair_channel = vim.fn.sockconnect("pipe", "/tmp/elm-pair/socket",
                                             {rpc = true})

    _G.elm_pair_on_elm_buffer()

    vim.cmd([[
      augroup elm-pair
        autocmd FileType elm call v:lua.elm_pair_on_elm_buffer()
      augroup END
    ]])
end

return {start = start}
