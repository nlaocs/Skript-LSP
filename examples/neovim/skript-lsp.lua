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

local config = {
  cmd = { exe, "--stdio" },
  root_dir = root,
  filetypes = { "skript" },
  init_options = { snapshotPath = snapshot, addonPaths = addons },
}

vim.filetype.add({ extension = { sk = "skript" } })
vim.lsp.config("skript_lsp", config)
vim.lsp.enable("skript_lsp")

return config
