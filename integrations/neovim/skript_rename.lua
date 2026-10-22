-- Strict preflight for Skript's versioned rename edits. Neovim's default
-- workspace edit helper skips version 0 and versions after the first entry.
local M = {}
local method = 'textDocument/rename'
local function available()
  local ok, tracking = pcall(require, 'vim.lsp._changetracking')
  return ok and type(tracking.flush) == 'function'
end
function M.capabilities(capabilities)
  capabilities.workspace = capabilities.workspace or {}
  capabilities.workspace.workspaceEdit = capabilities.workspace.workspaceEdit or {}
  capabilities.workspace.workspaceEdit.documentChanges = available()
end
local function prune(client, state)
  for buffer in pairs(state.epochs) do
    if not client.attached_buffers[buffer] then state.epochs[buffer] = nil end
  end
end
local function snapshot(client, state)
  prune(client, state)
  local tracking = require('vim.lsp._changetracking')
  for buffer in pairs(client.attached_buffers) do tracking.flush(client, buffer) end
  local result = {}
  for buffer in pairs(client.attached_buffers) do
    if vim.api.nvim_buf_is_valid(buffer) and vim.api.nvim_buf_is_loaded(buffer) then
      result[vim.uri_from_bufnr(buffer)] = {
        buffer = buffer, epoch = state.epochs[buffer], version = vim.lsp.util.buf_versions[buffer],
        tick = vim.api.nvim_buf_get_changedtick(buffer),
      }
    end
  end
  return { documents = result, lifetime = state.lifetime }
end
local function invalid(client, state, before, edit)
  if client:is_stopped() then return 'the client stopped' end
  if before.lifetime ~= state.lifetime then return 'the open document set changed' end
  if edit.changes or type(edit.documentChanges) ~= 'table' then
    return 'versioned documentChanges are required'
  end
  for _, change in ipairs(edit.documentChanges) do
    local document = change.textDocument
    if change.kind or type(document) ~= 'table' then return 'unsupported resource edit' end
    local old = before.documents[document.uri]
    if not old or not client.attached_buffers[old.buffer]
      or state.epochs[old.buffer] ~= old.epoch
      or not vim.api.nvim_buf_is_valid(old.buffer)
      or not vim.api.nvim_buf_is_loaded(old.buffer)
      or vim.uri_from_bufnr(old.buffer) ~= document.uri then
      return 'an edit target was closed, reopened or replaced'
    end
    if type(document.version) ~= 'number' or document.version ~= old.version
      or vim.lsp.util.buf_versions[old.buffer] ~= old.version
      or vim.api.nvim_buf_get_changedtick(old.buffer) ~= old.tick then
      return 'an edit target changed while rename was pending'
    end
    if not vim.bo[old.buffer].modifiable then return 'an edit target is not modifiable' end
  end
end
function M.attach(client, buffer)
  if not available() then return end
  local state = client._skript_static_rename
  if not state then
    state = { epochs = {}, counter = 0, lifetime = 0 }
    client._skript_static_rename = state
    -- Reload/save-as send close/open without another on_attach. Observe the
    -- actual transport synchronously; a deferred LspNotify event is too late.
    local notify = client.rpc.notify
    client.rpc.notify = function(notified, ...)
      if notified == 'textDocument/didOpen' or notified == 'textDocument/didClose' then
        state.lifetime = state.lifetime + 1
      end
      return notify(notified, ...)
    end
    local request = client.request
    client.request = function(self, requested, params, handler, bufnr)
      -- Neovim supports both client:request(...) and client.request(...).
      if self ~= client then
        self, requested, params, handler, bufnr = client, self, requested, params, handler
      end
      if requested ~= method then return request(self, requested, params, handler, bufnr) end
      local before = snapshot(self, state)
      handler = handler or self.handlers[method] or vim.lsp.handlers[method]
      return request(self, requested, params, function(err, edit, context)
        if not err and edit then
          local reason = invalid(self, state, before, edit)
          if reason then
            err, edit = { code = -32801, message = 'Rename cancelled: ' .. reason }, nil
          end
        end
        -- The builtin rename handler applies synchronously here. Custom handlers
        -- that defer application must revalidate again at their application time.
        return handler(err, edit, context)
      end, bufnr)
    end
  end
  prune(client, state)
  state.counter = state.counter + 1
  state.epochs[buffer] = state.counter
end
return M
