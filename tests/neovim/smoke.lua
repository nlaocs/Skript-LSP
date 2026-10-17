-- Run with Neovim 0.12+, -u NONE -i NONE -n --noplugin --headless -l.
-- This only edits in-memory buffers; the named .sk file is never written.
local api, uv = vim.api, vim.uv
local report = {
  schemaVersion = 1,
  status = "failed",
  neovim = vim.version(),
  checks = {},
  notifications = {},
  publications = {},
  timings = {},
}
local client, client_id, buffer, output, uri
local asynchronous_error, exited
local started = uv.hrtime()
local phase = "setup"
local function elapsed_ms()
  return (uv.hrtime() - started) / 1000000
end

local function wait_for(label, predicate, timeout)
  assert(elapsed_ms() < 90000, "overall smoke deadline exceeded during " .. label)
  local remaining = math.max(0, math.min(timeout or 30000, 90000 - elapsed_ms()))
  local value
  local ok = vim.wait(math.floor(remaining), function()
    if exited and phase ~= "shutdown" then
      asynchronous_error = "server exited during " .. phase .. ": " .. vim.inspect(exited)
    end
    if asynchronous_error then
      return true
    end
    value = predicate()
    return value ~= nil and value ~= false
  end, 10)
  assert(not asynchronous_error, asynchronous_error)
  assert(elapsed_ms() < 90000, "overall smoke deadline exceeded during " .. label)
  assert(ok and value, label .. " timed out")
  return value
end

local function notification(method, after)
  return wait_for(method, function()
    for index = after + 1, #report.notifications do
      local event = report.notifications[index]
      if event.method == method and event.params.textDocument.uri == uri then
        return event
      end
    end
  end)
end

local function publication(label, after, version, nonempty)
  return wait_for(label, function()
    for index = after + 1, #report.publications do
      local event = report.publications[index]
      if event.uri == uri and event.version == version and (#event.diagnostics > 0) == nonempty then
        return event
      end
    end
  end)
end

local function check(name, details)
  report.checks[#report.checks + 1] = { name = name, elapsedMs = elapsed_ms(), details = details }
end

local valid = { "on load:", '    send "hello" to console' }
local invalid = { "on load:", '    send "hello" to console', "!! skript_lsp_smoke_unclaimed !!" }

local function set_source(lines)
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
end

local function change(lines)
  local before = #report.notifications
  set_source(lines)
  local event = notification("textDocument/didChange", before)
  local changes = event.params.contentChanges
  assert(#changes == 1 and changes[1].range == nil, "Neovim must send full-text changes")
  assert(changes[1].text == table.concat(lines, "\n") .. "\n", "didChange must contain the complete buffer")
  return event.params.textDocument.version
end

local function settle_empty(after, version)
  local began = uv.hrtime()
  wait_for("latest diagnostics remain empty", function()
    for index = after + 1, #report.publications do
      local event = report.publications[index]
      if event.uri == uri then
        assert(event.version == version, "obsolete diagnostics arrived after the latest publication")
        assert(#event.diagnostics == 0, "nonempty diagnostics arrived after the latest empty publication")
      end
    end
    return (uv.hrtime() - began) / 1000000 >= 500
  end, 1000)
end

local function open_buffer(path, lines)
  buffer = vim.fn.bufadd(path)
  vim.fn.bufload(buffer)
  api.nvim_set_current_buf(buffer)
  vim.bo[buffer].swapfile = false
  vim.bo[buffer].undofile = false
  vim.bo[buffer].fileformat = "unix"
  vim.bo[buffer].endofline = true
  vim.bo[buffer].filetype = "skript"
  set_source(lines)
end

local function close_buffer(version)
  local sent, received = #report.notifications, #report.publications
  api.nvim_buf_delete(buffer, { force = true })
  buffer = nil
  notification("textDocument/didClose", sent)
  local cleared = publication("close clears diagnostics", received, version, false)
  settle_empty(cleared.sequence, version)
end

local function run()
  assert(vim.fn.has("nvim-0.12") == 1, "Neovim 0.12+ is required")
  assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT and vim.env.SKRIPT_LSP_SMOKE_OUTPUT ~= "",
    "SKRIPT_LSP_SMOKE_OUTPUT must name the workspace JSON result file")
  output = vim.fs.normalize(vim.fs.abspath(vim.env.SKRIPT_LSP_SMOKE_OUTPUT))
  local output_dir = vim.fs.dirname(output)
  assert(vim.fn.isdirectory(output_dir) == 1, "create the output directory before running the smoke test")
  -- Keep runtime state and logs beside the explicitly selected result file.
  vim.env.XDG_STATE_HOME = output_dir .. "/nvim-state"
  vim.env.XDG_CACHE_HOME = output_dir .. "/nvim-cache"
  vim.env.XDG_DATA_HOME = output_dir .. "/nvim-data"
  vim.env.NVIM_LOG_FILE = output_dir .. "/nvim.log"
  vim.opt.runtimepath = { vim.env.VIMRUNTIME }
  vim.opt.packpath = {}
  vim.o.modeline = false
  vim.o.shadafile = "NONE"

  local script = vim.fs.abspath(debug.getinfo(1, "S").source:sub(2))
  local checkout = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(script)))
  local config = dofile(checkout .. "/examples/neovim/skript-lsp.lua")
  vim.lsp.enable("skript_lsp", false)
  config.name = "skript_lsp_smoke"
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  config.detached = false
  config.exit_timeout = 2000
  config.capabilities = vim.lsp.protocol.make_client_capabilities()
  config.capabilities.general = config.capabilities.general or {}
  config.capabilities.general.positionEncodings = { "utf-16" }
  config.on_init = function(_, result)
    report.serverCapabilities = vim.deepcopy(result.capabilities)
  end
  config.on_error = function(code, message)
    asynchronous_error = "LSP client error " .. tostring(code) .. ": " .. tostring(message)
  end
  config.on_exit = function(code, signal)
    exited = { code = code, signal = signal }
  end
  local default_diagnostics = vim.lsp.handlers["textDocument/publishDiagnostics"]
  local default_log = vim.lsp.handlers["window/logMessage"]
  config.handlers = {
    ["window/logMessage"] = function(err, result, context, handler_config)
      local text = type(result) == "table" and result.message
      local encoded = type(text) == "string" and (text:match("^skript%-lsp timing (.+)$") or text:match("^skript%-lsp startup (.+)$"))
      if encoded and #report.timings < 256 then
        local ok, timing = pcall(vim.json.decode, encoded)
        if ok then
          timing.elapsedMs, timing.phase = elapsed_ms(), phase
          report.timings[#report.timings + 1] = timing
        end
      end
      if default_log then default_log(err, result, context, handler_config) end
    end,
    ["textDocument/publishDiagnostics"] = function(err, result, context, handler_config)
      if err or type(result) ~= "table" or type(result.version) ~= "number"
        or type(result.diagnostics) ~= "table" then
        asynchronous_error = "invalid or unversioned publishDiagnostics: " .. vim.inspect(err or result)
        return
      end
      if #report.publications >= 256 then
        asynchronous_error = "publishDiagnostics observation limit exceeded"
        return
      end
      local event = vim.deepcopy(result)
      event.phase, event.elapsedMs = phase, elapsed_ms()
      event.sequence = #report.publications + 1
      report.publications[#report.publications + 1] = event
      local ok, message = pcall(default_diagnostics, err, result, context, handler_config)
      if not ok then
        asynchronous_error = tostring(message)
      end
    end,
  }
  api.nvim_create_autocmd("LspNotify", {
    callback = function(event)
      local data = event.data
      if data.client_id ~= client_id or not data.params or not data.params.textDocument then
        return
      end
      if #report.notifications >= 256 then
        asynchronous_error = "LspNotify observation limit exceeded"
        return
      end
      report.notifications[#report.notifications + 1] = {
        method = data.method, params = vim.deepcopy(data.params), phase = phase, elapsedMs = elapsed_ms(),
      }
    end,
  })

  report.server = { executable = config.cmd[1], root = config.root_dir, initializationOptions = config.init_options }
  local document = config.root_dir .. "/skript-lsp-smoke-" .. uv.os_getpid() .. "-" .. uv.hrtime() .. ".sk"
  assert(not uv.fs_stat(document), "smoke document path already exists")
  uri = vim.uri_from_fname(document)
  report.documentUri = uri
  phase = "invalid-open"
  open_buffer(document, invalid)
  client_id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
  client = assert(vim.lsp.get_client_by_id(client_id))
  local opened = notification("textDocument/didOpen", 0)
  local version = opened.params.textDocument.version
  publication("invalid document diagnostics", 0, version, true)
  local sync = report.serverCapabilities.textDocumentSync
  assert((type(sync) == "table" and sync.change or sync) == 1, "server must advertise full synchronization")
  assert(client.offset_encoding == "utf-16", "server must use UTF-16 positions")
  check("invalid-open", { version = version, positionEncoding = client.offset_encoding })

  phase = "invalid-to-valid"
  local received = #report.publications
  version = change(valid)
  publication("valid document clears diagnostics", received, version, false)
  check("invalid-to-valid", { version = version })

  phase = "utf16"
  local unicode_line = "診断😀_skript_lsp_smoke_unclaimed"
  received = #report.publications
  version = change({ valid[1], valid[2], unicode_line })
  local unicode_result = publication("UTF-16 diagnostic", received, version, true)
  local utf16_end = vim.str_utfindex(unicode_line, "utf-16")
  local observed
  for _, diagnostic in ipairs(unicode_result.diagnostics) do
    local ending = diagnostic.range and diagnostic.range["end"]
    if ending and ending.line == 2 and ending.character == utf16_end then
      observed = diagnostic.range
      break
    end
  end
  assert(observed, "unclaimed Unicode line must end at its UTF-16 column, not its UTF-8 byte length")
  check("utf16", { version = version, utf16End = utf16_end, utf8Bytes = #unicode_line, range = observed })

  phase = "rapid-changes"
  local sent = #report.notifications
  received = #report.publications
  local versions = {}
  for index = 1, 6 do
    versions[index] = change(index % 2 == 1 and invalid or valid)
    if index > 1 then
      assert(versions[index] > versions[index - 1], "didChange versions must increase")
    end
  end
  version = versions[#versions]
  local latest = publication("latest rapid edit clears diagnostics", received, version, false)
  settle_empty(latest.sequence, version)
  check("rapid-changes", { versions = versions, sentNotifications = #report.notifications - sent })

  phase = "close"
  version = change(invalid)
  close_buffer(version)
  check("close", { clearedVersion = version })

  phase = "reopen"
  sent, received = #report.notifications, #report.publications
  open_buffer(document, valid)
  assert(vim.lsp.buf_attach_client(buffer, client_id), "reopen must attach to the same LSP client")
  opened = notification("textDocument/didOpen", sent)
  version = opened.params.textDocument.version
  assert(version == 0, "a fresh Neovim buffer must reopen this URI at version zero")
  local reopened = publication("reopened document clears diagnostics", received, version, false)
  settle_empty(reopened.sequence, version)
  check("reopen", { version = version })
  close_buffer(version)

  phase = "shutdown"
  client:stop(false)
  wait_for("server shutdown", function() return exited end, 5000)
  assert(exited.code == 0 and exited.signal == 0, "server did not exit normally")
  check("shutdown", exited)
end

local ok, failure = xpcall(run, debug.traceback)
if client and not exited then
  local stopped, stop_error = pcall(client.stop, client, true)
  if not stopped then
    ok, failure = false, tostring(failure or "") .. "\n" .. tostring(stop_error)
  end
  if not vim.wait(5000, function() return exited ~= nil end, 10) then
    ok, failure = false, tostring(failure or "") .. "\nserver cleanup timed out"
  end
end
report.status = ok and "passed" or "failed"
report.error = ok and vim.NIL or tostring(failure)
report.serverExit = exited or vim.NIL
report.elapsedMs = elapsed_ms()
if output then
  local file, message = io.open(output, "wb")
  if file then
    local written, write_error = file:write(vim.json.encode(report), "\n")
    local closed, close_error = file:close()
    if not written or not closed then
      ok, failure = false, write_error or close_error or "could not write smoke JSON"
    end
  else
    ok, failure = false, message
  end
end
if not ok then
  io.stderr:write(tostring(failure), "\n")
end
vim.cmd(ok and "qall!" or "cquit 1")
