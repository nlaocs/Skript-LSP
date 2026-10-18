-- Blink's supported LSP transform_items hook. Unknown items remain unchanged.
-- Positions stay in the LSP encoding; Blink performs its normal conversion.
local function position(value)
  return type(value) == "table"
    and type(value.line) == "number" and value.line >= 0 and value.line % 1 == 0
    and type(value.character) == "number" and value.character >= 0 and value.character % 1 == 0
end

return function(_, items)
  for index, item in ipairs(items) do
    local data = type(item.data) == "table" and item.data["nlaocs.lsp.completion-edit"]
    local edit = type(data) == "table" and data.version == 1 and data.edit
    local previous = item.textEdit
    local range = type(edit) == "table" and edit.range
    local before = type(previous) == "table" and previous.range
    if item.insertTextFormat == 1
      and type(range) == "table" and position(range.start) and position(range["end"])
      and type(before) == "table" and position(before.start) and position(before["end"])
      and range.start.line == range["end"].line
      and range.start.line == before.start.line and range.start.line == before["end"].line
      and range.start.character == before.start.character
      and range["end"].character >= before["end"].character
      and before["end"].character >= before.start.character
      and type(edit.newText) == "string" and not edit.newText:find("[\r\n]")
      and type(previous.newText) == "string" and edit.newText:sub(1, #previous.newText) == previous.newText
      and (item.additionalTextEdits == nil
        or (type(item.additionalTextEdits) == "table" and next(item.additionalTextEdits) == nil))
    then
      local replacement = vim.deepcopy(item)
      replacement.textEdit = vim.deepcopy(edit)
      items[index] = replacement
    end
  end
  return items
end
