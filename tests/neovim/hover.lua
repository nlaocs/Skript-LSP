-- Neovim 0.12+: --headless -u NONE -i NONE -n --noplugin -l this-file.lua.
-- Uses the existing SKRIPT_LSP_* smoke environment. Never writes a .sk file.
-- Every UI assertion observes vim.lsp.buf.hover()'s real floating buffer;
-- the response observer forwards the stock callback without changing it.
local api, uv = vim.api, vim.uv
local report = {
  schemaVersion = 1, status = "failed", neovim = vim.version(),
  cases = {}, requests = {}, notifications = {}, publications = {},
}
local client, client_id, buffer, output, uri, document, version
local exited, asynchronous_error, original_request
local phase, source = "setup", {}
local owned_floats = {}
local started = uv.hrtime()
local function elapsed()
  return (uv.hrtime() - started) / 1000000
end

local function wait_for(label, predicate, timeout)
  assert(elapsed() < 90000, "overall hover deadline exceeded: " .. label)
  local value
  local ok = vim.wait(math.floor(math.min(timeout or 30000, 90000 - elapsed())), function()
    if exited and phase ~= "shutdown" then asynchronous_error = "server exited: " .. vim.inspect(exited) end
    if asynchronous_error then return true end
    value = predicate()
    return value ~= nil and value ~= false
  end, 10)
  assert(not asynchronous_error, asynchronous_error)
  assert(ok and value and elapsed() < 90000, "deadline exceeded: " .. label)
  return value
end

local function sent(method, after)
  return wait_for(method, function()
    for index = after + 1, #report.notifications do
      local event = report.notifications[index]
      if event.method == method and event.params.textDocument.uri == uri then return event end
    end
  end)
end

local function published(after, wanted_version, nonempty)
  return wait_for("publishDiagnostics version " .. wanted_version, function()
    for index = after + 1, #report.publications do
      local event = report.publications[index]
      if event.uri == uri and event.version == wanted_version and (#event.diagnostics > 0) == nonempty then return event end
    end
  end)
end

local function close_floats()
  for win in pairs(owned_floats) do
    if api.nvim_win_is_valid(win) then api.nvim_win_close(win, true) end
  end
  owned_floats = {}
end

local function float_snapshot()
  local result = {}
  for _, win in ipairs(api.nvim_list_wins()) do
    local config = api.nvim_win_get_config(win)
    if config.relative ~= "" then
      local buf = api.nvim_win_get_buf(win)
      result[#result + 1] = {
        window = win, buffer = buf, relative = config.relative,
        sourceBuffer = vim.w[win].lsp_floating_bufnr or vim.NIL,
        width = config.width, height = config.height, syntax = vim.bo[buf].syntax,
        lines = api.nvim_buf_get_lines(buf, 0, -1, false),
      }
      -- An isolated -u NONE process has no unrelated floats. Still clean up
      -- only the previews explicitly associated with our own source buffer.
      if vim.w[win].lsp_floating_bufnr == buffer then owned_floats[win] = true end
    end
  end
  return result
end

local function position(row, byte_col)
  return { line = row, character = vim.str_utfindex(source[row + 1], "utf-16", byte_col) }
end

local function at(row, text, offset)
  local index = assert(source[row + 1]:find(text, 1, true), "cursor token absent from fixture: " .. text)
  return index - 1 + (offset or 0)
end

local function before(a, b)
  return a.line < b.line or a.line == b.line and a.character < b.character
end

local function selected_source(range)
  local first, last = range.start, range["end"]
  assert(type(first) == "table" and type(last) == "table" and before(first, last), "hover range must be nonempty and half-open")
  assert(first.line >= 0 and last.line < #source, "hover range must refer to the current source")
  local start_byte = vim.str_byteindex(source[first.line + 1], "utf-16", first.character, false)
  local end_byte = vim.str_byteindex(source[last.line + 1], "utf-16", last.character, false)
  assert(vim.str_utfindex(source[first.line + 1], "utf-16", start_byte) == first.character
    and vim.str_utfindex(source[last.line + 1], "utf-16", end_byte) == last.character,
    "hover range contains an out-of-bounds column or splits a UTF-16 surrogate pair")
  local lines = {}
  for row = first.line, last.line do
    local text = source[row + 1]
    lines[#lines + 1] = text:sub(row == first.line and start_byte + 1 or 1, row == last.line and end_byte or #text)
  end
  return table.concat(lines, "\n"), { start = { row = first.line, byte = start_byte }, ["end"] = { row = last.line, byte = end_byte } }
end

local function observe_requests()
  original_request = client.request
  client.request = function(self, method, params, handler, bufnr)
    if method ~= "textDocument/hover" then return original_request(self, method, params, handler, bufnr) end
    assert(#report.requests < 128, "hover observation limit exceeded")
    local event = {
      sequence = #report.requests + 1, phase = phase, params = vim.deepcopy(params),
      sentMs = elapsed(), publicationsBeforeRequest = #report.publications, completed = false,
    }
    report.requests[#report.requests + 1] = event
    return original_request(self, method, params, function(err, result, context, config)
      event.error, event.result = err or vim.NIL, result or vim.NIL
      event.receivedMs, event.requestId = elapsed(), context.request_id
      event.contextVersion = context.version or vim.NIL
      -- Keep the original result and standard hover callback untouched.
      local ok, message = pcall(handler, err, result, context, config)
      if not ok then asynchronous_error = tostring(message) end
      event.completed, event.completedMs = true, elapsed()
    end, bufnr)
  end
end

local function hover(row, byte_col, expect_null, required)
  close_floats()
  api.nvim_set_current_buf(buffer)
  api.nvim_win_set_cursor(0, { row + 1, byte_col })
  vim.cmd("redraw")
  local expected_position = position(row, byte_col)
  local after = #report.requests
  vim.lsp.buf.hover({ silent = true, focus = false, border = "single" })
  local event = wait_for("standard hover response", function()
    local candidate = report.requests[after + 1]
    return candidate and candidate.completed and candidate
  end, 5000)
  local observation = {
    requestSequence = event.sequence, expectedPosition = expected_position,
    source = vim.deepcopy(source), expectedNull = expect_null,
  }
  -- Store before any assertions so a failing UI still leaves useful evidence.
  local record = report.cases[#report.cases]
  record.hovers[#record.hovers + 1] = observation
  vim.cmd("redraw")
  observation.floats = float_snapshot()
  if event.error ~= vim.NIL then
    assert(#observation.floats == 0, "an LSP error response produced a floating window")
    error("hover returned an LSP error: " .. vim.inspect(event.error))
  end
  assert(vim.deep_equal(event.params.position, expected_position), "Neovim hover cursor was not encoded as UTF-16")
  assert(event.params.textDocument.uri == uri, "hover used a different document URI")
  if expect_null then
    assert(event.result == vim.NIL, "expected null hover, received " .. vim.inspect(event.result))
    assert(#observation.floats == 0, "a null hover produced a floating window")
    observation.verified = true
    return observation, event
  end
  local result = event.result
  assert(type(result) == "table" and type(result.contents) == "table", "hover must return MarkupContent")
  assert(result.contents.kind == "plaintext" and type(result.contents.value) == "string" and result.contents.value ~= "",
    "hover must return nonempty PlainText MarkupContent")
  assert(type(result.range) == "table", "hover must return the selected source range")
  local first, last = result.range.start, result.range["end"]
  assert(not before(expected_position, first) and before(expected_position, last), "hover range does not contain the requested character")
  observation.selectedSource, observation.byteRange = selected_source(result.range)
  assert(#observation.floats == 1, "standard hover did not create exactly one real floating window")
  local float = observation.floats[1]
  assert(float.sourceBuffer == buffer, "floating window does not belong to the current source buffer")
  local expected_lines = vim.split(result.contents.value, "\n", { trimempty = true })
  observation.expectedFloatLines = expected_lines
  assert(vim.deep_equal(float.lines, expected_lines), "real floating-buffer text differs from the actual PlainText response")
  assert(float.syntax == "plaintext", "standard hover did not use the plaintext renderer")
  for _, text in ipairs(required or {}) do
    assert(result.contents.value:find(text, 1, true), "hover is missing expected information: " .. text)
  end
  local ns = api.nvim_get_namespaces()["nvim.lsp.hover_range"]
  observation.rangeExtmarks = ns and api.nvim_buf_get_extmarks(buffer, ns, 0, -1, { details = true }) or {}
  local highlighted = false
  for _, mark in ipairs(observation.rangeExtmarks) do
    local details = mark[4]
    if mark[2] == observation.byteRange.start.row and mark[3] == observation.byteRange.start.byte
      and details.end_row == observation.byteRange["end"].row and details.end_col == observation.byteRange["end"].byte then
      highlighted = true
    end
  end
  assert(highlighted, "standard hover extmarks do not match the returned UTF-16 source range")
  observation.verified = true
  return observation, event
end

local function set_source(lines)
  close_floats()
  source = vim.deepcopy(lines)
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
end

local function changed(after)
  local event = sent("textDocument/didChange", after)
  local changes = event.params.contentChanges
  assert(#changes == 1 and changes[1].range == nil, "Neovim must send full-text changes")
  assert(changes[1].text == table.concat(source, "\n") .. "\n", "didChange does not contain the current complete source")
  version = event.params.textDocument.version
  return event
end

local function change(lines, nonempty)
  local sent_before, received_before = #report.notifications, #report.publications
  set_source(lines)
  changed(sent_before)
  local publication = published(received_before, version, nonempty)
  local record = report.cases[#report.cases]
  record.diagnosticPublications[#record.diagnosticPublications + 1] = publication.sequence
  return publication
end

local function open_buffer(lines)
  buffer = vim.fn.bufadd(document)
  vim.fn.bufload(buffer)
  api.nvim_set_current_buf(buffer)
  vim.bo[buffer].swapfile, vim.bo[buffer].undofile = false, false
  vim.bo[buffer].fileformat, vim.bo[buffer].endofline, vim.bo[buffer].filetype = "unix", true, "skript"
  set_source(lines)
end

local function close_buffer()
  close_floats()
  local sent_before, received_before = #report.notifications, #report.publications
  api.nvim_buf_delete(buffer, { force = true })
  buffer = nil
  sent("textDocument/didClose", sent_before)
  return published(received_before, version, false)
end

local failures = 0
local function case(name, callback)
  phase = name
  local record = { name = name, status = "failed", hovers = {}, diagnosticPublications = {} }
  report.cases[#report.cases + 1] = record
  local ok, error_message = xpcall(function() callback(record) end, debug.traceback)
  if ok then record.status = "passed" else record.error = tostring(error_message); failures = failures + 1 end
  close_floats()
  assert(not asynchronous_error and not exited, asynchronous_error or "server exited during hover regression")
end

local function read_function(name)
  local path = vim.fs.normalize(vim.fs.abspath(vim.env.SKRIPT_LSP_SNAPSHOT)) .. "/Functions.json"
  local file = assert(io.open(path, "rb"))
  local text = assert(file:read("*a"))
  assert(file:close())
  for _, entry in ipairs(vim.json.decode(text)) do
    if entry.name == name and entry.addon.name == "Skript" then
      assert(entry.returnType == "java.lang.Number" and #entry.parameters == 1 and entry.parameters[1].type == "java.lang.Number",
        "fixture builtin signature changed; update this evidence-backed case")
      return entry, "Signature: " .. name .. "(" .. entry.parameters[1].name .. ": number) -> number"
    end
  end
  error("fixture does not contain the required Skript builtin function: " .. name)
end

local valid = { "on load:", '    send "hello" to console' }
local unknown = { "on load:", "    this effect does not exist" }
local function run()
  assert(vim.fn.has("nvim-0.12") == 1, "Neovim 0.12+ is required")
  assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT and vim.env.SKRIPT_LSP_SMOKE_OUTPUT ~= "", "SKRIPT_LSP_SMOKE_OUTPUT is required")
  output = vim.fs.normalize(vim.fs.abspath(vim.env.SKRIPT_LSP_SMOKE_OUTPUT))
  local directory = vim.fs.dirname(output)
  assert(vim.fn.isdirectory(directory) == 1, "create the workspace output directory first")
  vim.env.XDG_STATE_HOME, vim.env.XDG_CACHE_HOME = directory .. "/nvim-state", directory .. "/nvim-cache"
  vim.env.XDG_DATA_HOME, vim.env.NVIM_LOG_FILE = directory .. "/nvim-data", directory .. "/nvim.log"
  vim.opt.runtimepath, vim.opt.packpath = { vim.env.VIMRUNTIME }, {}
  vim.o.modeline, vim.o.shadafile = false, "NONE"
  report.renderer = {
    source = vim.env.VIMRUNTIME .. "/lua/vim/lsp/buf.lua", method = "vim.lsp.buf.hover",
    observation = "Real standard hover floating window, buffer text and source-range extmarks; no replacement renderer.",
  }
  report.limitations = {
    "Client publication timing does not prove the worker's pre-parse state; deterministic readiness belongs to controlled-worker server tests.",
    "No intentionally faulting WASM Addon is injected by this real-snapshot UI test; fault exclusion requires server/parser tests.",
    "The close/reopen test observes the same URI and a reset version; delayed old-worker completion requires controlled-worker tests.",
  }
  local script = vim.fs.abspath(debug.getinfo(1, "S").source:sub(2))
  local checkout = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(script)))
  local config = dofile(checkout .. "/examples/neovim/skript-lsp.lua")
  vim.lsp.enable("skript_lsp", false)
  config.name, config.detached, config.exit_timeout = "skript_lsp_hover_regression", false, 2000
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  config.capabilities = vim.lsp.protocol.make_client_capabilities()
  config.capabilities.general = config.capabilities.general or {}
  config.capabilities.general.positionEncodings = { "utf-16" }
  config.capabilities.textDocument.hover.contentFormat = { "plaintext" }
  config.on_init = function(_, result) report.serverCapabilities = vim.deepcopy(result.capabilities) end
  config.on_error = function(code, message) asynchronous_error = "LSP error " .. code .. ": " .. tostring(message) end
  config.on_exit = function(code, signal) exited = { code = code, signal = signal } end
  local default_diagnostics = vim.lsp.handlers["textDocument/publishDiagnostics"]
  config.handlers = {
    ["textDocument/publishDiagnostics"] = function(err, result, context, handler_config)
      if err or type(result) ~= "table" or type(result.version) ~= "number" or type(result.diagnostics) ~= "table" then
        asynchronous_error = "invalid or unversioned diagnostics: " .. vim.inspect(err or result); return
      end
      if #report.publications >= 256 then asynchronous_error = "publication observation limit exceeded"; return end
      local event = vim.deepcopy(result)
      event.sequence, event.phase, event.elapsedMs = #report.publications + 1, phase, elapsed()
      report.publications[#report.publications + 1] = event
      local ok, message = pcall(default_diagnostics, err, result, context, handler_config)
      if not ok then asynchronous_error = tostring(message) end
    end,
  }
  api.nvim_create_autocmd("LspNotify", { callback = function(event)
    local data = event.data
    if data.client_id ~= client_id or not data.params or not data.params.textDocument then return end
    if #report.notifications >= 256 then asynchronous_error = "notification observation limit exceeded"; return end
    report.notifications[#report.notifications + 1] = {
      method = data.method, params = vim.deepcopy(data.params), phase = phase, elapsedMs = elapsed(),
    }
  end })
  report.server = { executable = config.cmd[1], root = config.root_dir, initializationOptions = config.init_options }
  document = config.root_dir .. "/skript-lsp-hover-" .. uv.os_getpid() .. "-" .. uv.hrtime() .. ".sk"
  assert(not uv.fs_stat(document), "unique memory document already exists")
  uri = vim.uri_from_fname(document)
  report.documentUri = uri
  open_buffer(valid)
  client_id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
  client = assert(vim.lsp.get_client_by_id(client_id))
  version = sent("textDocument/didOpen", 0).params.textDocument.version
  published(0, version, false)
  assert(report.serverCapabilities.hoverProvider, "server must advertise hover")
  assert(client.offset_encoding == "utf-16", "server must negotiate UTF-16")
  assert(#vim.lsp.get_clients({ bufnr = buffer }) == 1, "hover fixture must have only its own client")
  observe_requests()

  case("adopted-send-effect", function()
    hover(1, at(1, "send", 1), false, { "Effect:", "Pattern:", "Addon: Skript 2.15.4" })
  end)
  case("unicode-string-utf16", function(record)
    change({ "on load:", '    send "日本😀語" to console' }, false)
    local observed, event = hover(1, at(1, "語"), false, { "Return type: string", "Multiplicity: single" })
    assert(observed.selectedSource:find("日本😀語", 1, true), "hover did not select the Unicode string expression")
    local ending = event.result.range["end"]
    record.unicode = { requestUtf16 = event.params.position.character, cursorByte = at(1, "語"),
      rangeEndUtf16 = ending.character, rangeEndByte = observed.byteRange["end"].byte }
    assert(record.unicode.requestUtf16 ~= record.unicode.cursorByte, "Unicode fixture did not distinguish UTF-16 from bytes")
    assert(record.unicode.rangeEndUtf16 ~= record.unicode.rangeEndByte, "returned Unicode range was not checked against UTF-8 bytes")
  end)
  case("variable-local-global-context", function()
    change({ "on load:", "    send {_money} to console", "    send {money} to console" }, false)
    for _, query in ipairs({ { row = 1, token = "{_money}", scope = "local" }, { row = 2, token = "{money}", scope = "global" } }) do
      local _, event = hover(query.row, at(query.row, query.token, 2), false,
        { "Variable", "Scope: " .. query.scope, "Name template: money", "Type (parse context, not inferred): " })
      assert(event.result.contents.value:match("Type %(parse context, not inferred%): [^\n]+"), "variable parse-context type must be explicit")
    end
  end)
  case("builtin-function-registry-signature", function(record)
    local entry, signature = read_function("sin")
    record.registryEvidence = entry
    change({ "on load:", "    send sin(1) to console" }, false)
    hover(1, at(1, "sin", 1), false, { "Function: sin", signature, "Return type: number", "Addon: Skript " .. entry.addon.version })
  end)
  case("partial-document-adopted-sibling", function()
    change({ "on load:", valid[2], unknown[2] }, true)
    hover(1, at(1, "send", 1), false, { "Effect:", "Pattern:" })
    hover(2, at(2, "effect", 1), true)
  end)
  case("whitespace-comment-blank-null", function()
    change({ "on load:", valid[2], "    # hover has no semantic target here", "" }, false)
    hover(1, 1, true)
    hover(2, at(2, "semantic", 1), true)
    hover(3, 0, true)
  end)
  case("edit-invalidates-hover-and-diagnostics", function(record)
    change({ "on load:", "    send sin(1) to console" }, false)
    hover(1, at(1, "sin", 1), false, { "Function: sin" })
    local sent_before, received_before = #report.notifications, #report.publications
    set_source(unknown)
    -- The standard client flushes didChange before issuing the hover request.
    -- Unknown input has no accepted AST both before and after its parse, so a
    -- non-null response here would expose stale information in either state.
    local _, event = hover(1, at(1, "effect", 1), true)
    changed(sent_before)
    local seen_before_request = false
    for index = received_before + 1, event.publicationsBeforeRequest do
      local publication = report.publications[index]
      if publication.uri == uri and publication.version == version then seen_before_request = true end
    end
    record.readinessObservation = { version = version, currentVersionPublicationSeenAtRequest = seen_before_request,
      preParseStateProven = false, nullRequestSequence = event.sequence }
    local invalid = published(received_before, version, true)
    record.diagnosticPublications[#record.diagnosticPublications + 1] = invalid.sequence
    change({ "on load:", "    send cos(1) to console" }, false)
    local _, corrected = hover(1, at(1, "cos", 1), false, { "Function: cos" })
    assert(not corrected.result.contents.value:find("Function: sin", 1, true), "old function hover survived correction")
  end)
  case("close-reopen-same-uri-generation", function(record)
    local old_version = version
    record.closePublicationSequence = close_buffer().sequence
    local complete, response
    assert(client:request("textDocument/hover", { textDocument = { uri = uri }, position = { line = 1, character = 10 } },
      function(err, result) response = { error = err or vim.NIL, result = result or vim.NIL }; complete = true end, 0))
    wait_for("closed URI hover", function() return complete end, 5000)
    record.closedHover = response
    assert(response.error == vim.NIL and response.result == vim.NIL, "closed document retained hover data")
    local sent_before, received_before = #report.notifications, #report.publications
    open_buffer({ "on load:", "    send {_money} to console" })
    assert(vim.lsp.buf_attach_client(buffer, client_id), "reopened URI must use the same owned client")
    version = sent("textDocument/didOpen", sent_before).params.textDocument.version
    record.oldVersion, record.reopenedVersion = old_version, version
    assert(version == 0 and old_version > 0, "reopened memory buffer must reset the URI version to zero")
    local reopened = published(received_before, version, false)
    record.diagnosticPublications[#record.diagnosticPublications + 1] = reopened.sequence
    local _, event = hover(1, at(1, "_money", 1), false, { "Variable", "Scope: local" })
    assert(not event.result.contents.value:find("Function: cos", 1, true), "the previous document generation leaked into reopened hover")
  end)
  phase = "close"
  if buffer and api.nvim_buf_is_valid(buffer) then close_buffer() end
  phase = "shutdown"
  client.request = original_request
  client:stop(false)
  wait_for("normal server exit", function() return exited end, 5000)
  assert(exited.code == 0 and exited.signal == 0, "server did not exit normally")
  assert(failures == 0 and #report.cases == 8, tostring(failures) .. " hover regression case(s) failed")
end

local ok, failure = xpcall(run, debug.traceback)
pcall(close_floats)
if client and not exited then
  if original_request then client.request = original_request end
  local stopped, message = pcall(client.stop, client, true)
  if not stopped then ok, failure = false, tostring(failure or "") .. "\n" .. tostring(message) end
  if not vim.wait(5000, function() return exited ~= nil end, 10) then
    ok, failure = false, tostring(failure or "") .. "\nowned server cleanup timed out"
  end
end
report.status, report.error = ok and "passed" or "failed", ok and vim.NIL or tostring(failure)
report.serverExit, report.elapsedMs = exited or vim.NIL, elapsed()
if output then
  local file, message = io.open(output, "wb")
  if file then
    local written, write_error = file:write(vim.json.encode(report), "\n")
    local closed, close_error = file:close()
    if not written or not closed then ok, failure = false, write_error or close_error or "JSON write failed" end
  else ok, failure = false, message end
end
if not ok then io.stderr:write(tostring(failure), "\n") end
vim.cmd(ok and "qall!" or "cquit 1")
