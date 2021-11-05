-- luacheck: read globals vim
local function start()
    local socket = assert(io.open("/tmp/elm-pair", "a"))

    local function on_bytes(_, buffer, _, start_row, start_col, start_byte,
                            old_row, old_col, old_byte, new_row, new_col,
                            new_byte)
        local old_end_byte = start_byte + old_byte
        local new_end_byte = start_byte + new_byte
        local old_end_row = start_row + old_row
        local new_end_row = start_row + new_row
        local old_end_col = old_col + ((old_row == 0) and start_col or 0)
        local new_end_col = new_col + ((new_row == 0) and start_col or 0)
        local filename = vim.fn.expand("%:p")
        -- getbufline takes 1-indexed rows, whereas nvim_buf_attach passes us
        -- 0-indexed rows.
        local changed_lines = vim.fn.getbufline(buffer, start_row + 1,
                                                new_end_row + 1)
        local msg = {
            filename, changed_lines, start_byte, old_end_byte, new_end_byte,
            start_row, start_col, old_end_row, old_end_col, new_end_row,
            new_end_col
        }
        socket:write(vim.fn.json_encode(msg))
        socket:write("\n")
        socket:flush()
    end

    -- Send the entire buffer when we start. Most of the arguments below are
    -- dummies, but we need to report row data correctly (because this is used
    -- to select the right rows from buffer).
    on_bytes("bytes", vim.fn.bufnr("%"), "init", 0, 0, 0, 0, 0, 0,
             vim.fn.line("$"), 0, 0)

    vim.api.nvim_buf_attach(0, false, {on_bytes = on_bytes})
end

return {start = start}
