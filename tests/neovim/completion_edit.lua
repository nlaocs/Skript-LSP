-- Run with nvim --headless -u NONE -l tests/neovim/completion_edit.lua.
local transform = dofile(vim.env.SKRIPT_COMPLETION_EDIT_ADAPTER or "examples/neovim/lsp_completion_edit.lua")
local fallback = {
  label = "sin(argument)", kind = 3, insertTextFormat = 1,
  textEdit = { range = { start = { line = 1, character = 9 }, ["end"] = { line = 1, character = 11 } }, newText = "si" },
  data = { unrelated = "preserved", ["nlaocs.lsp.completion-edit"] = { version = 1,
    edit = { range = { start = { line = 1, character = 9 }, ["end"] = { line = 1, character = 13 } }, newText = "sin(" } } },
}
local snapshot = vim.deepcopy(fallback)
local ordinary = { label = "other_lsp_function", kind = 3, insertTextFormat = 1 }
local items = transform({}, { fallback, ordinary })
assert(items[2] == ordinary, "ordinary items must retain identity and fields")
assert(items[1].kind == 3 and items[1].data.unrelated == "preserved")
assert(vim.deep_equal(fallback, snapshot), "original item must not be mutated")
assert(vim.deep_equal(items[1].textEdit, fallback.data["nlaocs.lsp.completion-edit"].edit))
local first_transform = vim.deepcopy(items)
transform({}, items)
assert(vim.deep_equal(items, first_transform), "repeat transforms must be stable")
for _, change in ipairs({
  function(i) i.data["nlaocs.lsp.completion-edit"].version = 2 end,
  function(i) i.data = {} end,
  function(i) i.insertTextFormat = 2 end,
  function(i) i.data["nlaocs.lsp.completion-edit"].edit.range["end"].character = -1 end,
  function(i) i.data["nlaocs.lsp.completion-edit"].edit.range.start.character = 8 end,
  function(i) i.data["nlaocs.lsp.completion-edit"].edit.newText = "different(" end,
  function(i) i.data["nlaocs.lsp.completion-edit"].edit.newText = "sin(\n" end,
  function(i) i.data["nlaocs.lsp.completion-edit"].edit = false end,
  function(i) i.additionalTextEdits = { i.textEdit } end,
  function(i) i.additionalTextEdits = false end,
}) do
  local item = vim.deepcopy(fallback)
  change(item)
  local before = vim.deepcopy(item)
  assert(transform({}, { item })[1] == item and vim.deep_equal(item, before), "invalid/unknown data must keep the fallback")
end
-- No encoding conversion: these are UTF-16 columns, even beyond byte columns
-- used by a client for display. Blink owns the eventual conversion.
local unicode = vim.deepcopy(fallback)
unicode.textEdit.range.start.character = 20
unicode.textEdit.range["end"].character = 22
unicode.data["nlaocs.lsp.completion-edit"].edit.range.start.character = 20
unicode.data["nlaocs.lsp.completion-edit"].edit.range["end"].character = 24
assert(transform({}, { unicode })[1].textEdit.range["end"].character == 24)
print("completion edit adapter: passed")
