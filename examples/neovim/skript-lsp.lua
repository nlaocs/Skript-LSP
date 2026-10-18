-- Neovim 0.12+. Load with -u for an isolated session, or :luafile in an
-- existing session before opening a .sk file. No config files are rewritten.
local function absolute_path(value, label, expected_type)
  assert(type(value) == "string" and value ~= "", label .. " is required")
  local path = vim.fs.normalize(vim.fs.abspath(value))
  local stat = vim.uv.fs_stat(path)
  assert(stat and stat.type == expected_type, label .. " must name a " .. expected_type .. ": " .. path)
  return path
end

local exe = absolute_path(vim.env.SKRIPT_LSP_EXE, "SKRIPT_LSP_EXE", "file")
local snapshot = absolute_path(vim.env.SKRIPT_LSP_SNAPSHOT, "SKRIPT_LSP_SNAPSHOT", "directory")
local root = absolute_path(vim.env.SKRIPT_LSP_ROOT, "SKRIPT_LSP_ROOT", "directory")
local addons = {}
if vim.env.SKRIPT_LSP_ADDON_PATHS and vim.env.SKRIPT_LSP_ADDON_PATHS ~= "" then
  addons = vim.json.decode(vim.env.SKRIPT_LSP_ADDON_PATHS)
  assert(type(addons) == "table" and vim.islist(addons), "SKRIPT_LSP_ADDON_PATHS must be a JSON array")
  for index, path in ipairs(addons) do
    addons[index] = absolute_path(path, "addonPaths[" .. index .. "]", "file")
  end
end

-- Neovim can render arbitrary token names, but its default capability list
-- contains only standard names. Explicitly opt in to Skript's syntax roles.
local capabilities = vim.lsp.protocol.make_client_capabilities()
for _, name in ipairs({ "skriptEvent", "skriptCondition", "skriptEffect", "skriptExpression",
  "skriptSection", "skriptStructure", "skriptLiteral", "skriptInterpolationDelimiter" }) do
  table.insert(capabilities.textDocument.semanticTokens.tokenTypes, name)
end
-- Match the REPL's source-syntax RGB palette; keep literal text theme-aware.
for role, color in pairs({
  skriptEvent = "#C55F73", skriptCondition = "#FC6255", skriptEffect = "#58C4DD",
  skriptExpression = "#83C167", skriptSection = "#5CD0B3", skriptStructure = "#9A72AC",
  variable = "#00FFFF", ["function"] = "#A0A0A0",
  skriptInterpolationDelimiter = "#B0B0B0",
}) do
  vim.api.nvim_set_hl(0, "@lsp.type." .. role .. ".skript", { fg = color, default = true })
end
vim.api.nvim_set_hl(0, "@lsp.type.skriptLiteral.skript", { link = "Normal", default = true })

local config = {
  cmd = { exe, "--stdio" },
  root_dir = root,
  filetypes = { "skript" },
  init_options = { snapshotPath = snapshot, addonPaths = addons },
  capabilities = capabilities,
  on_attach = function(client, bufnr)
    if client:supports_method("textDocument/completion", bufnr) then
      vim.opt_local.completeopt = { "menu", "menuone", "noselect" }
      vim.lsp.completion.enable(true, client.id, bufnr, { autotrigger = true })
      vim.keymap.set("i", "<C-Space>", vim.lsp.completion.get,
        { buffer = bufnr, desc = "Skript completion" })
    end
  end,
}

vim.filetype.add({ extension = { sk = "skript" } })
vim.lsp.config("skript_lsp", config)
vim.lsp.enable("skript_lsp")

return config
