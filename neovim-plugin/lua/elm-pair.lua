-- luacheck: read globals vim
local function start()
    local socket = assert(io.open("/tmp/elm-pair", "a"))
    local buffer = vim.fn.bufnr("%")

    local function send_json_msg(msg)
        socket:write(vim.fn.json_encode(msg))
        socket:write("\n")
        socket:flush()
    end

    local function on_bytes(_, _, _, start_row, start_col, start_byte, old_row,
                            old_col, old_byte, new_row, new_col, new_byte)
        local old_end_byte = start_byte + old_byte
        local new_end_byte = start_byte + new_byte
        local old_end_row = start_row + old_row
        local new_end_row = start_row + new_row
        local old_end_col = old_col + ((old_row == 0) and start_col or 0)
        local new_end_col = new_col + ((new_row == 0) and start_col or 0)
        local filename = vim.fn.expand("%:p")
        -- getbufline takes 1-indexed rows, whereas nvim_buf_attach passes us
        -- 0-indexed rows.
        local changed_lines = table.concat(
                                  vim.fn.getbufline(buffer, start_row + 1,
                                                    new_end_row + 1), "\n")
        local changed_code = changed_lines:sub(1 + start_col,
                                               start_col + new_byte)
        local msg = {
            filename, changed_code, start_byte, old_end_byte, new_end_byte,
            start_row, start_col, old_end_row, old_end_col, new_end_row,
            new_end_col
        }
        send_json_msg(msg)
    end

    -- Send the entire buffer when we start.
    send_json_msg({
        vim.fn.expand("%:p"),
        table.concat(vim.fn.getbufline(buffer, 1, 1 + vim.fn.line("$")), "\n"),
        -- The arguments below are all dummies. They don't matter for the
        -- initial send.
        0, 0, 0, 0, 0, 0, 0, 0, 0
    })

    vim.api.nvim_buf_attach(buffer, false, {on_bytes = on_bytes})
end

return {start = start}
