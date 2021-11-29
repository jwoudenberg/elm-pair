-- luacheck: read globals vim
local function start()
    local channel = vim.fn.sockconnect("pipe", "/tmp/elm-pair.sock",
                                       {rpc = true})
    local buffer = vim.fn.bufnr("%")
    local path = vim.fn.expand("%:p")
    vim.fn.rpcnotify(channel, "buffer_opened", buffer, path)
end

return {start = start}
