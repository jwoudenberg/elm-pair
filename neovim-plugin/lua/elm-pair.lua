local function start()
  socket = assert(io.open("/tmp/elm-pair", "a"))

  local function on_bytes(_, bufnr, changed_tick, start_row, start_col, start_byte, old_row, old_col, old_byte, new_row, new_col, new_byte)
    local old_end_col = old_col + ((old_row == 0) and start_col or 0)
    local new_end_col = new_col + ((new_row == 0) and start_col or 0)
    local filename = vim.fn.expand("%:p")
    local msg = {
      filename,
      start_byte,
      (start_byte+old_byte),
      (start_byte+new_byte),
      start_row,
      start_col,
      (start_row+old_row),
      old_end_col,
      (start_row+new_row),
      new_end_col,
    }
    socket:write(vim.fn.json_encode(msg))
    socket:write("\n")
    socket:flush()
  end

  vim.api.nvim_buf_attach(0, false, {on_bytes=on_bytes})
end

return { start = start, }
