-- Neovim 0.12 stock completion menu and CompleteDone edits, isolated by caller.
-- The unique .sk URI is backed only by an in-memory buffer.
local api, uv = vim.api, vim.uv
local report = { schemaVersion = 1, status = "failed", cases = {}, requests = {}, timings = {}, publications = {} }
local client, buffer, output, exited, asynchronous_error
local started = uv.hrtime()
local function save(phase)
  report.phase = phase
  if output then
    local file = assert(io.open(output, "wb")); file:write(vim.json.encode(report), "\n"); file:close()
  end
end
local function elapsed() return (uv.hrtime() - started) / 1000000 end
local function wait(label, predicate, timeout)
  local value
  assert(vim.wait(timeout or 30000, function()
    if asynchronous_error then return true end
    value = predicate()
    return value ~= nil and value ~= false
  end, 10), label .. " timed out")
  assert(not asynchronous_error, asynchronous_error)
  return value
end
local function check(name, details) report.cases[#report.cases + 1] = { name = name, details = details } end
local function update(lines)
  local before = #report.publications
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
  local changed = api.nvim_buf_get_changedtick(buffer)
  wait("current revision diagnostics", function()
    for i = before + 1, #report.publications do
      if report.publications[i].version == changed then return true end
    end
  end)
end
local function menu(name, row, column, label, expected, keys)
  if vim.snippet.active() then vim.snippet.stop() end
  api.nvim_feedkeys(vim.keycode("<Esc><Esc>"), "nx", false)
  save(name .. ": before input")
  api.nvim_set_current_buf(buffer)
  api.nvim_win_set_cursor(0, { row, column })
  local began, snapshot, failure = elapsed(), nil, nil
  local timer = uv.new_timer()
  timer:start(10, 10, vim.schedule_wrap(function()
    vim.cmd("redraw")
    if elapsed() - began > 15000 then
      failure = "stock completion menu timed out: " .. name
      timer:stop()
      api.nvim_feedkeys(vim.keycode("<Esc>"), "nt", false)
      return
    end
    if vim.fn.pumvisible() == 1 then
      local info = vim.fn.complete_info({ "items", "selected" })
      if not report.lastObservedMenu then
        report.lastObservedMenu = vim.deepcopy(info)
        save(name .. ": menu inspect")
      end
      for i, item in ipairs(info.items) do
        local original = vim.tbl_get(item, "user_data", "nvim", "lsp", "completion_item")
        if original and label(original.label, original) then
          snapshot = vim.deepcopy(info)
          report.pendingMenu = snapshot
          save(name .. ": menu observed")
          timer:stop()
          local steps = i - 1 - info.selected
          if steps < 0 then steps = steps + #info.items end
          api.nvim_feedkeys(vim.keycode(string.rep("<C-n>", steps) .. "<C-y><Esc>"), "nt", false)
          return
        end
      end
    end
  end))
  -- x! intentionally lets the real input loop process LSP replies. The timer
  -- always queues Esc on success or deadline; no fixed sleep drives the test.
  api.nvim_feedkeys(vim.keycode(keys or "a<C-Space>"), "mtx!", false)
  api.nvim_feedkeys(vim.keycode("<Esc><Esc>"), "nx", false)
  save(name .. ": input returned")
  timer:stop(); timer:close()
  assert(not failure, failure)
  assert(snapshot, "target absent from actual menu: " .. name)
  local line = api.nvim_buf_get_lines(buffer, row - 1, row, false)[1]
  check(name, { menu = snapshot, line = line, requestToAcceptMs = elapsed() - began })
  assert(expected(line), "unexpected accepted buffer: " .. line)
  if vim.snippet.active() then vim.snippet.stop() end
end
local function run()
  output = assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT)
  save("setup")
  vim.opt.runtimepath = { vim.env.VIMRUNTIME }; vim.opt.packpath = {}
  vim.o.modeline = false; vim.o.shadafile = "NONE"
  local script = vim.fs.abspath(debug.getinfo(1, "S").source:sub(2))
  local root = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(script)))
  local config = dofile(vim.env.SKRIPT_LSP_CONFIGURATION or (root .. "/examples/neovim/skript-lsp.lua"))
  vim.lsp.enable("skript_lsp", false)
  config.name = "skript_lsp_completion_smoke"
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  config.detached = false; config.exit_timeout = 2000
  config.on_exit = function(code, signal) exited = { code = code, signal = signal } end
  config.on_error = function(code, message) asynchronous_error = tostring(code) .. ": " .. message end
  local diagnostic_handler = vim.lsp.handlers["textDocument/publishDiagnostics"]
  config.handlers = {
    ["textDocument/publishDiagnostics"] = function(err, result, ctx, options)
      assert(not err)
      report.publications[#report.publications + 1] = vim.deepcopy(result)
      diagnostic_handler(err, result, ctx, options)
    end,
    ["window/logMessage"] = function(_, result)
      local text = result.message
      local data = text:match("^skript%-lsp completion timing (.+)$")
        or text:match("^skript%-lsp timing (.+)$") or text:match("^skript%-lsp startup (.+)$")
      if data then report.timings[#report.timings + 1] = vim.json.decode(data) end
    end,
  }
  local path = config.root_dir .. "/skript-lsp-completion-" .. uv.os_getpid() .. "-" .. uv.hrtime() .. ".sk"
  assert(not uv.fs_stat(path))
  buffer = vim.fn.bufadd(path); vim.fn.bufload(buffer); api.nvim_set_current_buf(buffer)
  vim.bo[buffer].swapfile = false; vim.bo[buffer].undofile = false; vim.bo[buffer].filetype = "skript"
  api.nvim_buf_set_lines(buffer, 0, -1, false, { "on load:", "    send si" })
  local id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
  client = assert(vim.lsp.get_client_by_id(id))
  local request = client.request
  client.request = function(self, method, params, handler, bufnr)
    if method ~= "textDocument/completion" then return request(self, method, params, handler, bufnr) end
    local event = { params = vim.deepcopy(params), sentMs = elapsed() }
    report.requests[#report.requests + 1] = event
    return request(self, method, params, function(err, result, ctx, options)
      event.error, event.result, event.receivedMs = err or vim.NIL, result or vim.NIL, elapsed()
      handler(err, result, ctx, options)
    end, bufnr)
  end
  wait("initial diagnostics", function() return #report.publications > 0 end)
  save("initial diagnostics received")
  assert(client.server_capabilities.completionProvider.resolveProvider == false)
  assert(vim.fn.maparg("<C-Space>", "i") ~= "", "sample installs buffer-local invocation")
  menu("function snippet", 2, 10, function(label) return label:match("^sin%(") end,
    function(line) return line:match("^    send sin%([^%)]*%)$") and not line:find("${", 1, true) end)
  update({ "on load:", "    send {_money} to console", "    send {_mo} to console" })
  menu("existing variable closing brace", 3, 12, function(label) return label == "{_money}" end,
    function(line) return line == "    send {_money} to console" end)
  update({ "on load:", "    cancel " })
  menu("statement after whitespace", 2, 10, function(_, item) return item.textEdit.newText == "cancel event" end,
    function(line) return line:find("cancel event", 1, true) ~= nil end)
  update({ "on load:", "    send name of player" })
  menu("existing fixed suffix", 2, 10, function(label) return label:find("name", 1, true) ~= nil end,
    function(line) return line == "    send name of player" end)
  update({ "on load:", "    send {_money} to console", "    send " })
  local before_trigger = #report.requests
  menu("brace autotrigger", 3, 8, function(label) return label == "{_money}" end,
    function(line) return line == "    send {_money}" end, "a{")
  assert(#report.requests > before_trigger)
  assert(report.requests[before_trigger + 1].params.context.triggerKind == 2)
  assert(report.requests[before_trigger + 1].params.context.triggerCharacter == "{")
  -- Optional external-addon qualification; the caller supplies matched data.
  -- Default Core-only smoke remains independent of any specific addon binary.
  if vim.env.SKRIPT_LSP_REFLECT_SMOKE == "1" then
    update({ "on load:", '    set {_name} to "hello"', "    set {_copy} to {_name}", "    set {_part} to {_copy}.sub" })
    local line = api.nvim_buf_get_lines(buffer, 3, 4, false)[1]
    menu("reflect copied receiver snippet", 4, #line - 1,
      function(_, item) return item.filterText == "substring" and item.label == "substring(int)" end,
      function(accepted) return accepted:match("^    set {_part} to {_copy}%.substring%([^%)]*%)$") and not accepted:find("${", 1, true) end)
    update({ "on load:", '    set {_😀} to "hello"', "    set {_part} to ({_😀}).sub(" })
    line = api.nvim_buf_get_lines(buffer, 2, 3, false)[1]
    -- Cursor is before the existing parenthesis; Neovim converts byte to UTF-16.
    menu("reflect Unicode existing parenthesis", 3, #line - 2,
      function(_, item) return item.filterText == "substring" and item.label == "substring(int)" end,
      function(accepted) return accepted == "    set {_part} to ({_😀}).substring(" end)
  end
  if vim.env.SKRIPT_LSP_GUARD_SMOKE == "1" then
    local function guarded_menu(name, lines, copy_row)
      update(lines)
      local response = assert(client:request_sync("textDocument/inlayHint", {
        textDocument = { uri = vim.uri_from_bufnr(buffer) },
        range = { start = { line = 0, character = 0 }, ["end"] = { line = #lines - 1, character = #lines[#lines] } },
      }, 30000, buffer))
      assert(not response.err, vim.inspect(response.err))
      local found = false
      for _, hint in ipairs(response.result or {}) do
        if hint.position.line == copy_row - 1 then
          assert(hint.label == ": string", "guard must remove nullable only inside checked branch: " .. vim.inspect(hint))
          found = true
        end
      end
      assert(found, "guarded copy inlay missing")
      check(name .. " type", { hints = response.result })
      menu(name, #lines, #lines[#lines] - 1,
        function(_, item) return item.filterText == "substring" and item.label == "substring(int)" end,
        function(accepted) return accepted:match("^        set {_part} to {_copy}%.substring%([^%)]*%)$") and not accepted:find("$" .. "{", 1, true) end)
    end
    guarded_menu("presence guard actual insertion", {
      "function guarded(name: text):", "    if {_name} is set:",
      "        set {_copy} to {_name}", "        set {_part} to {_copy}.sub",
    }, 3)
    guarded_menu("mixed instanceof actual insertion", {
      "import:", "    java.lang.String as JavaString", "on load:",
      '    set {_name} to "hello"', "    if chance of 50%:", "        set {_name} to 123",
      "    if {_name} is an instance of JavaString:", "        set {_copy} to {_name}", "        set {_part} to {_copy}.sub",
    }, 8)
  end
  if vim.env.SKRIPT_LSP_CUSTOM_EXPRESSION_SMOKE == "1" then
    local lines = { "# 雪😀", "local expression review greeting:", "    return type: string",
      "    get:", '        return "hello"', "on load:", "    set {_message} to review gr" }
    update(lines)
    menu("custom expression multiword insertion", 7, #lines[7] - 1,
      function(label) return label == "review greeting" end,
      function(accepted) return accepted == "    set {_message} to review greeting" end)
    wait("custom expression accepted revision", function()
      local tick = api.nvim_buf_get_changedtick(buffer)
      for _, publication in ipairs(report.publications) do if publication.version == tick then return true end end
    end)
    local hints = assert(client:request_sync("textDocument/inlayHint", {
      textDocument = { uri = vim.uri_from_bufnr(buffer) },
      range = { start = { line = 0, character = 0 }, ["end"] = { line = 6,
        character = #api.nvim_buf_get_lines(buffer, 6, 7, false)[1] } },
    }, 30000, buffer))
    assert(not hints.err, vim.inspect(hints.err))
    local found = false
    for _, hint in ipairs(hints.result or {}) do
      if hint.position.line == 6 then
        assert(hint.label == ": string?", "declared getter type must retain possible empty return: " .. vim.inspect(hint))
        found = true
      end
    end
    assert(found, "custom expression stored type hint missing")
    check("custom expression nullable stored type", { hints = hints.result })
    lines[7] = "    set {_message} to review greeting"
    lines[8] = "    set {_part} to {_message}.sub"
    update(lines)
    menu("custom expression stored receiver insertion", 8, #lines[8] - 1,
      function(_, item) return item.filterText == "substring" and item.label:match("^substring%(int%)") end,
      function(accepted) return accepted:match("^    set {_part} to {_message}%.substring%([^%)]*%)$") and not accepted:find("${", 1, true) end)
  end
  check("request observations", { count = #report.requests, timings = #report.timings })
  assert(#report.requests >= 4)
  client:stop(false)
  wait("shutdown", function() return exited end, 5000)
  assert(exited.code == 0 and exited.signal == 0)
end
vim.schedule(function()
local ok, failure = xpcall(run, debug.traceback)
if client and not exited then client:stop(true); vim.wait(5000, function() return exited end, 10) end
report.status = ok and "passed" or "failed"; report.error = ok and vim.NIL or tostring(failure)
report.elapsedMs = elapsed(); report.serverExit = exited or vim.NIL
if output then
  local file = assert(io.open(output, "wb")); file:write(vim.json.encode(report), "\n"); file:close()
end
if not ok then io.stderr:write(tostring(failure), "\n") end
vim.cmd(ok and "qall!" or "cquit 1")
end)
