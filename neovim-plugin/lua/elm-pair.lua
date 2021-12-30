-- luacheck: read globals vim
local function on_elm_buffer(buffer, path)
    vim.fn.rpcnotify(_G.elm_pair_channel, "buffer_opened", buffer, path)
end

function _G.elm_pair_on_elm_buffer()
    local buffer = vim.fn.bufnr("%")
    local path = vim.fn.expand("%:p")
    on_elm_buffer(buffer, path)
end

local function connect_to_socket(socket_path)
    _G.elm_pair_channel = vim.fn.sockconnect("pipe", socket_path, {rpc = true})

    -- Tell elm-pair about Elm buffers opened while elm-pair was starting.
    for _, buf in pairs(vim.api.nvim_list_bufs()) do
        if vim.api.nvim_buf_is_loaded(buf) and
            vim.fn.getbufvar(buf, "&filetype") == "elm" then
            local info = vim.fn.getbufinfo(buf)[1]
            on_elm_buffer(buf, info.name)
        end
    end

    -- Ensure elm-pair learns about Elm buffers we might open later.
    vim.cmd([[
        augroup elm-pair
        autocmd FileType elm call v:lua.elm_pair_on_elm_buffer()
        augroup END
    ]])
end

local function start()
    local stderr
    local job_id = vim.fn.jobstart({"elm-pair"}, {
        stdout_buffered = true,
        on_stdout = function(_, data, _)
            connect_to_socket(vim.fn.join(data))
        end,
        stderr_buffered = true,
        on_stderr = function(_, data, _) stderr = vim.fn.join(data) end,
        on_exit = function(_, exit_code, _)
            if exit_code > 0 then
                error(
                    "`elm-pair` failed with exit code " .. exit_code .. ": " ..
                        stderr)
            end
        end
    });
    if job_id <= 0 then error("calling `elm-pair` failed: " .. job_id) end
end

return {start = start}
