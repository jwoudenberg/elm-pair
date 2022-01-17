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

local function elm_pair_binary()
    if vim.fn.executable("elm-pair") > 0 then
        return "elm-pair"
    else
        local script_path = debug.getinfo(2, "S").source:sub(2):match("(.*/)")
        return script_path .. "../elm-pair"
    end
end

local function start()
    local stdout
    local job_id = vim.fn.jobstart({elm_pair_binary()}, {
        stdout_buffered = true,
        on_stdout = function(_, data, _) stdout = vim.fn.join(data) end,
        on_stderr = function(_, data, _) print(vim.fn.join(data)) end,
        on_exit = function(_, exit_code, _)
            if exit_code == 0 then
                connect_to_socket(stdout)
            else
                error("`elm-pair` failed with exit code " .. exit_code)
            end
        end
    });
    if job_id <= 0 then error("calling `elm-pair` failed: " .. job_id) end
end

return {start = start}
