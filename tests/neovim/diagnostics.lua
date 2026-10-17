-- Neovim 0.12: --headless -u NONE -i NONE -n --noplugin -l this-file.lua
-- Uses the same SKRIPT_LSP_* variables as smoke.lua. No .sk file is written.
-- Synthetic renderer controls are recorded separately from real LSP cases.
local api, uv = vim.api, vim.uv
local report = {
  schemaVersion = 1, status = "failed", neovim = vim.version(),
  controls = {}, cases = {}, notifications = {}, publications = {},
}
local client, client_id, buffer, output, uri, namespace, exited, async_error
local phase = "setup"
local started = uv.hrtime()
local function elapsed()
  return (uv.hrtime() - started) / 1000000
end

local function wait_for(label, predicate, timeout)
  assert(elapsed() < 90000, "overall deadline exceeded: " .. label)
  local value
  local ok = vim.wait(math.floor(math.min(timeout or 30000, 90000 - elapsed())), function()
    if exited and phase ~= "shutdown" then async_error = "server exited: " .. vim.inspect(exited) end
    if async_error then return true end
    value = predicate()
    return value ~= nil and value ~= false
  end, 10)
  assert(not async_error, async_error)
  assert(ok and value and elapsed() < 90000, "deadline exceeded: " .. label)
  return value
end

local function configure(ns)
  -- Only this process and this namespace. Keep the stock virtual-text handler,
  -- prefix, formatting and per-line message selection; never sort the payload.
  vim.diagnostic.config({
    virtual_text = true, virtual_lines = false, severity_sort = false,
    update_in_insert = true, signs = false, underline = false,
  }, ns)
end

local function flatten(chunks)
  local text = {}
  for _, chunk in ipairs(chunks) do text[#text + 1] = chunk[1] end
  return table.concat(text)
end

local function snapshot(ns, buf)
  vim.cmd("redraw")
  local metadata = vim.diagnostic.get_namespace(ns)
  -- Neovim 0.12's standard handler creates this namespace. Inspecting its
  -- extmarks is deliberate; calling _get_virt_text_chunks alone is not a UI test.
  local virtual_namespace = metadata.user_data.virt_text_ns
  local result = { namespace = ns, virtualTextNamespace = virtual_namespace or vim.NIL, diagnostics = {}, extmarks = {} }
  for _, diagnostic in ipairs(vim.diagnostic.get(buf, { namespace = ns })) do
    result.diagnostics[#result.diagnostics + 1] = {
      code = diagnostic.code, message = diagnostic.message, severity = diagnostic.severity,
      lnum = diagnostic.lnum, col = diagnostic.col,
      end_lnum = diagnostic.end_lnum, end_col = diagnostic.end_col,
      lsp = diagnostic.user_data and diagnostic.user_data.lsp or vim.NIL,
    }
  end
  local marks = virtual_namespace and api.nvim_buf_get_extmarks(buf, virtual_namespace, 0, -1, { details = true }) or {}
  for _, mark in ipairs(marks) do
    local chunks = mark[4].virt_text
    if chunks then
      result.extmarks[#result.extmarks + 1] = {
        id = mark[1], row = mark[2], col = mark[3], chunks = chunks,
        text = flatten(chunks), position = mark[4].virt_text_pos,
      }
    end
  end
  return result
end

local function inline_text(view, row)
  local text = {}
  for _, mark in ipairs(view.extmarks) do
    if mark.row == row then text[#text + 1] = mark.text end
  end
  return table.concat(text, "\n")
end

local function visible(view, row, message)
  local normalized = message:gsub("\r", ""):gsub("\n", "  ")
  return inline_text(view, row):find(normalized, 1, true) ~= nil
end

local function controls()
  local ns = api.nvim_create_namespace("skript-lsp-ui-renderer-controls")
  local buf = api.nvim_create_buf(false, true)
  api.nvim_set_current_buf(buf)
  api.nvim_buf_set_lines(buf, 0, -1, false, { "control row zero with separate columns", "control row one" })
  configure(ns)
  local function diagnostic(row, col, message)
    return { lnum = row, col = col, end_lnum = row, end_col = col + 1,
      severity = vim.diagnostic.severity.ERROR, message = message, source = "synthetic-renderer-control" }
  end
  local samples = {
    { name = "same-position", items = { diagnostic(0, 0, "concrete control"), diagnostic(0, 0, "summary control") }, last = "summary control" },
    { name = "different-columns-same-line", items = { diagnostic(0, 0, "concrete control"), diagnostic(0, 20, "summary control") }, last = "summary control" },
    { name = "reverse-order", items = { diagnostic(0, 20, "summary control"), diagnostic(0, 0, "concrete control") }, last = "concrete control" },
    { name = "separate-lines", items = { diagnostic(0, 0, "first child control"), diagnostic(1, 0, "second child control") } },
  }
  for _, sample in ipairs(samples) do
    vim.diagnostic.set(ns, buf, sample.items)
    local view = snapshot(ns, buf)
    local record = { name = sample.name, input = sample.items, view = view, status = "failed" }
    report.controls[#report.controls + 1] = record
    assert(#view.extmarks > 0, "renderer control did not create virtual-text extmarks")
    if sample.last then
      assert(visible(view, 0, sample.last), "stock virtual text did not show the last same-line diagnostic")
      local hidden = sample.last == "summary control" and "concrete control" or "summary control"
      assert(not visible(view, 0, hidden), "renderer control unexpectedly showed both message bodies")
    else
      assert(visible(view, 0, "first child control") and visible(view, 1, "second child control"),
        "separate diagnostic rows did not produce both inline messages")
    end
    record.status = "passed"
  end
  vim.diagnostic.reset(ns, buf)
  api.nvim_buf_delete(buf, { force = true })
end

local function sent(method, after)
  return wait_for(method, function()
    for index = after + 1, #report.notifications do
      local event = report.notifications[index]
      if event.method == method and event.params.textDocument.uri == uri then return event end
    end
  end)
end

local function published(version, after, predicate)
  return wait_for("publishDiagnostics version " .. version, function()
    for index = after + 1, #report.publications do
      local event = report.publications[index]
      if event.uri == uri and event.version == version and (not predicate or predicate(event)) then return event end
    end
  end)
end

local function summary(diagnostic)
  return diagnostic.code == "parser.structure.unclaimed" or diagnostic.code == "parser.section.unclaimed"
    or diagnostic.message == "Structure trigger child could not be parsed uniquely"
end

local cases = {
  { name = "else-rejection", lines = { "on load:", "    else:", "        send 1 to console" },
    specific_rows = { 1 }, expected_message = "'else' has to be placed just after another 'if' or 'else if' section" },
  { name = "invalid-effect-captures", lines = { "on load:", "    teleport a to location(b, 2, 3)" }, specific_rows = { 1 } },
  { name = "multiple-children", lines = { "on load:", "    teleport a to location(b, 2, 3)", "    teleport c to location(d, 2, 3)" }, specific_rows = { 1, 2 } },
  -- An unknown Command entry is rejected by EntryValidator before any typed
  -- value/capture parse. A top-level gibberish header is unsuitable: unrelated
  -- Structure attempts can contribute a Type-parser failure to that header.
  { name = "cause-free-fallback", lines = { "command /ui_fallback:", "    unrecognized entry: value", "    trigger:", "        send 1 to console" }, fallback_rows = { 1 } },
  { name = "specific-and-unrelated-fallback", lines = { "on load:", "    else:", "        send 1 to console", "command /ui_fallback:", "    unrecognized entry: value", "    trigger:", "        send 1 to console" },
    specific_rows = { 1 }, fallback_rows = { 4 } },
  { name = "same-range-unknown-effect", lines = { "on load:", "    this effect does not exist" }, specific_rows = { 1 },
    expected_message = "no WASM Type parser is registered for this SSG Type", expected_code = "parser.syntax-incomplete",
    expected_range = { start = { line = 1, character = 4 }, ["end"] = { line = 1, character = 30 } } },
}

local function verify_case(sample, event, record)
  local view = snapshot(namespace, buffer)
  record.view = view
  assert(#event.diagnostics > 0 and #view.extmarks > 0, "actual LSP errors did not produce inline extmarks")
  assert(#view.diagnostics == #event.diagnostics, "Neovim diagnostic cache lost or added LSP diagnostics")
  local used = {}
  for _, diagnostic in ipairs(view.diagnostics) do
    local match
    for index, raw in ipairs(event.diagnostics) do
      if not used[index] and vim.deep_equal(diagnostic.lsp, raw) then match = index; break end
    end
    assert(match, "cached diagnostic does not match the actual LSP publication")
    used[match], diagnostic.publicationIndex = true, match
  end
  if sample.expected_range then
    -- Assert the original UTF-16 LSP range, independently of Neovim's converted
    -- byte columns. Keep the evidence even if the old generic summary still
    -- competes with this cause and the following UI assertions fail.
    local observation = {
      expected = { code = sample.expected_code, message = sample.expected_message, range = vim.deepcopy(sample.expected_range) },
      matches = {}, sameRangeSummaries = {}, verified = false,
    }
    record.expectedDiagnosticObservation = observation
    for index, raw in ipairs(event.diagnostics) do
      if vim.deep_equal(raw.range, sample.expected_range) then
        local evidence = { publicationIndex = index, diagnostic = vim.deepcopy(raw) }
        if raw.code == sample.expected_code and raw.message == sample.expected_message then
          observation.matches[#observation.matches + 1] = evidence
        end
        if summary(raw) then observation.sameRangeSummaries[#observation.sameRangeSummaries + 1] = evidence end
      end
    end
    assert(#observation.matches == 1, "actual LSP publication must contain exactly one expected cause with the exact UTF-16 range")
    observation.verified = true
  end
  record.sameLinePositions = {}
  for _, row in ipairs(sample.specific_rows or {}) do
    local concrete, positions = {}, {}
    for _, diagnostic in ipairs(view.diagnostics) do
      if diagnostic.lnum == row and not summary(diagnostic) then
        concrete[#concrete + 1] = diagnostic
        positions[diagnostic.col .. ":" .. diagnostic.end_lnum .. ":" .. diagnostic.end_col] = true
      end
    end
    local count = vim.tbl_count(positions)
    record.sameLinePositions[#record.sameLinePositions + 1] = {
      row = row, concreteCount = #concrete, distinctRanges = count,
      multiplePositionsObserved = count > 1,
      limitation = count > 1 and vim.NIL or "This real input did not expose multiple concrete capture ranges; renderer controls cover distinct columns separately.",
    }
    assert(#concrete > 0, "specific cause missing on source row " .. row)
    for _, diagnostic in ipairs(view.diagnostics) do
      -- Each specified row belongs to exactly one failed Effect/Section child.
      -- This is a fixture-specific invariant, not a generic same-line filter.
      assert(not (diagnostic.lnum == row and summary(diagnostic)),
        "unclaimed summary still competes with the specific cause; changing publish order is insufficient")
    end
    local shown = false
    for _, diagnostic in ipairs(concrete) do
      if visible(view, row, diagnostic.message) then shown = true end
    end
    assert(shown, "inline chunks do not contain any corresponding concrete cause on row " .. row)
    if sample.expected_message then
      assert(visible(view, row, sample.expected_message), "the actual rejection reason is missing from inline chunks")
    end
  end
  for _, row in ipairs(sample.fallback_rows or {}) do
    local found = false
    for _, diagnostic in ipairs(view.diagnostics) do
      if diagnostic.lnum == row then
        assert(summary(diagnostic), "fallback fixture unexpectedly has a specific cause on row " .. row)
        if visible(view, row, diagnostic.message) then found = true end
      end
    end
    assert(found, "cause-free fallback was removed or is not visible on row " .. row)
  end
end

local function run()
  assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT and vim.env.SKRIPT_LSP_SMOKE_OUTPUT ~= "", "SKRIPT_LSP_SMOKE_OUTPUT is required")
  output = vim.fs.normalize(vim.fs.abspath(vim.env.SKRIPT_LSP_SMOKE_OUTPUT))
  local directory = vim.fs.dirname(output)
  assert(vim.fn.isdirectory(directory) == 1, "create the workspace output directory first")
  assert(vim.fn.has("nvim-0.12") == 1, "Neovim 0.12+ is required")
  vim.env.XDG_STATE_HOME, vim.env.XDG_CACHE_HOME = directory .. "/nvim-state", directory .. "/nvim-cache"
  vim.env.XDG_DATA_HOME, vim.env.NVIM_LOG_FILE = directory .. "/nvim-data", directory .. "/nvim.log"
  vim.opt.runtimepath, vim.opt.packpath = { vim.env.VIMRUNTIME }, {}
  vim.o.modeline, vim.o.shadafile = false, "NONE"
  report.renderer = {
    source = vim.env.VIMRUNTIME .. "/lua/vim/diagnostic.lua", virtualText = true, severitySort = false,
    observation = "Standard diagnostic handler extmarks and chunks, not a replacement renderer or a chunk-generator call.",
  }
  controls()
  local script = vim.fs.abspath(debug.getinfo(1, "S").source:sub(2))
  local checkout = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(script)))
  local config = dofile(checkout .. "/examples/neovim/skript-lsp.lua")
  vim.lsp.enable("skript_lsp", false)
  config.name, config.detached, config.exit_timeout = "skript_lsp_ui_regression", false, 2000
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  config.capabilities = vim.lsp.protocol.make_client_capabilities()
  config.capabilities.general = config.capabilities.general or {}
  config.capabilities.general.positionEncodings = { "utf-16" }
  config.on_init = function(_, result) report.serverCapabilities = result.capabilities end
  config.on_error = function(code, message) async_error = "LSP error " .. code .. ": " .. tostring(message) end
  config.on_exit = function(code, signal) exited = { code = code, signal = signal } end
  local default_handler = vim.lsp.handlers["textDocument/publishDiagnostics"]
  config.handlers = {
    ["textDocument/publishDiagnostics"] = function(err, result, context, handler_config)
      if err or type(result) ~= "table" or type(result.version) ~= "number" or type(result.diagnostics) ~= "table" then
        async_error = "invalid or unversioned diagnostics: " .. vim.inspect(err or result); return
      end
      if #report.publications >= 256 then async_error = "publication observation limit exceeded"; return end
      local event = vim.deepcopy(result)
      event.sequence, event.phase, event.elapsedMs = #report.publications + 1, phase, elapsed()
      report.publications[#report.publications + 1] = event
      if not namespace then namespace = vim.lsp.diagnostic.get_namespace(context.client_id); configure(namespace) end
      local ok, message = pcall(default_handler, err, result, context, handler_config)
      if not ok then async_error = tostring(message) end
    end,
  }
  api.nvim_create_autocmd("LspNotify", { callback = function(event)
    local data = event.data
    if data.client_id ~= client_id or not data.params or not data.params.textDocument then return end
    if #report.notifications >= 256 then async_error = "notification observation limit exceeded"; return end
    report.notifications[#report.notifications + 1] = {
      method = data.method, params = vim.deepcopy(data.params), phase = phase, elapsedMs = elapsed(),
    }
  end })
  report.server = { executable = config.cmd[1], root = config.root_dir, initializationOptions = config.init_options }
  local document = config.root_dir .. "/skript-lsp-ui-" .. uv.os_getpid() .. "-" .. uv.hrtime() .. ".sk"
  assert(not uv.fs_stat(document), "unique memory document already exists")
  buffer = api.nvim_create_buf(true, false)
  api.nvim_buf_set_name(buffer, document)
  api.nvim_set_current_buf(buffer)
  vim.bo[buffer].swapfile, vim.bo[buffer].undofile = false, false
  vim.bo[buffer].fileformat, vim.bo[buffer].endofline, vim.bo[buffer].filetype = "unix", true, "skript"
  uri = vim.uri_from_fname(document)
  report.documentUri = uri
  local failures, last_version = 0, nil
  for index, sample in ipairs(cases) do
    phase = sample.name
    local record = { name = sample.name, input = sample.lines, status = "failed" }
    report.cases[#report.cases + 1] = record
    local ok, message = xpcall(function()
      local before_sent, before_received = #report.notifications, #report.publications
      api.nvim_buf_set_lines(buffer, 0, -1, false, sample.lines)
      local method = "textDocument/didChange"
      if index == 1 then
        client_id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
        client = assert(vim.lsp.get_client_by_id(client_id))
        method = "textDocument/didOpen"
      end
      last_version = sent(method, before_sent).params.textDocument.version
      local event = published(last_version, before_received)
      record.version, record.publicationSequence = last_version, event.sequence
      assert(client.offset_encoding == "utf-16", "server did not negotiate UTF-16")
      verify_case(sample, event, record)
      record.status = "passed"
    end, debug.traceback)
    if not ok then record.error = tostring(message); failures = failures + 1 end
    if async_error or exited then break end
  end
  phase = "close"
  local before_sent, before_received = #report.notifications, #report.publications
  api.nvim_buf_delete(buffer, { force = true })
  buffer = nil
  sent("textDocument/didClose", before_sent)
  local cleared = published(assert(last_version), before_received, function(event) return #event.diagnostics == 0 end)
  assert(#cleared.diagnostics == 0, "close did not clear diagnostics")
  phase = "shutdown"
  client:stop(false)
  wait_for("normal server exit", function() return exited end, 5000)
  assert(exited.code == 0 and exited.signal == 0, "server did not exit normally")
  assert(failures == 0 and #report.cases == #cases, tostring(failures) .. " UI regression case(s) failed")
end

local ok, failure = xpcall(run, debug.traceback)
if client and not exited then
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
