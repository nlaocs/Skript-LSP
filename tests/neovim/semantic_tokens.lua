-- Neovim 0.12+: real semantic-token decoding and standard highlight extmarks.
-- Only in-memory .sk buffers and this run's output directory are written.
local api, uv = vim.api, vim.uv
local report = { schemaVersion = 1, status = "failed", checks = {}, publications = {}, responses = {} }
local client, client_id, buffer, output, uri, exited, asynchronous_error
local phase, lines = "setup", {}
local started = uv.hrtime()
local standard_only = vim.env.SKRIPT_LSP_STANDARD_TOKENS == "1"
local function wait_for(label, predicate)
  local value
  local ok = vim.wait(30000, function()
    if asynchronous_error then return true end
    if exited and phase ~= "shutdown" then asynchronous_error = "server exited: " .. vim.inspect(exited); return true end
    value = predicate()
    return value ~= nil and value ~= false
  end, 10)
  assert(not asynchronous_error, asynchronous_error)
  assert(ok and value and (uv.hrtime() - started) / 1e9 < 110, label .. " timed out")
  return value
end
local function token(row, text, wanted)
  local start = assert(lines[row + 1]:find(text, 1, true)) - 1
  local tokens = vim.lsp.semantic_tokens.get_at_pos(buffer, row, start) or {}
  for _, value in ipairs(tokens) do
    if value.type == wanted and value.start_col == start and value.end_col == start + #text then return value end
  end
end
local function at(row, text)
  local column = assert(lines[row + 1]:find(text, 1, true)) - 1
  -- get_at_pos uses a zero-width query, which can include the previous token's
  -- half-open end. Check ownership of this actual source character instead.
  return vim.tbl_filter(function(value)
    return value.line == row and value.start_col <= column and column < value.end_col
  end, vim.lsp.semantic_tokens.get_at_pos(buffer, row, column) or {})
end
local function inspect_mark(row, text, wanted)
  local column = assert(lines[row + 1]:find(text, 1, true)) - 1
  vim.cmd("redraw")
  for name, namespace in pairs(api.nvim_get_namespaces()) do
    if name:find("semantic", 1, true) then
      for _, mark in ipairs(api.nvim_buf_get_extmarks(buffer, namespace, { row, column }, { row, column }, { details = true })) do
        local details = mark[4]
        if details.hl_group == "@lsp.type." .. wanted .. ".skript" and details.end_col == column + #text then
          return { namespace = name, mark = mark, highlight = api.nvim_get_hl(0, { name = details.hl_group, link = false }) }
        end
      end
    end
  end
end
local function check(name, details)
  report.checks[#report.checks + 1] = { name = name, details = details or {} }
end
local function change(source, expect_error)
  lines = source
  local after = #report.publications
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
  return wait_for("analysis publication", function()
    for index = after + 1, #report.publications do
      local publication = report.publications[index]
      if publication.uri == uri then
        assert((#publication.diagnostics > 0) == expect_error, vim.inspect(publication))
        return publication
      end
    end
  end)
end
local ok, failure = xpcall(function()
  output = vim.env.SKRIPT_LSP_OUTPUT or vim.fs.dirname(assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT))
  vim.opt.runtimepath = { vim.env.VIMRUNTIME, vim.fs.normalize(vim.env.VIMRUNTIME .. "/../../../lib/nvim") }
  vim.opt.packpath = {}
  vim.o.modeline, vim.o.shadafile = false, "NONE"
  local script = vim.fs.abspath(debug.getinfo(1, "S").source:sub(2))
  local checkout = vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(script)))
  local configuration = vim.env.SKRIPT_LSP_CONFIGURATION or checkout .. "/examples/neovim/skript-lsp.lua"
  local config = dofile(configuration)
  report.configuration = configuration
  vim.lsp.enable("skript_lsp", false)
  config.name, config.detached, config.exit_timeout = "skript_lsp_semantic_regression", false, 2000
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  if standard_only then config.capabilities = vim.lsp.protocol.make_client_capabilities() end
  config.on_init = function(_, result) report.capabilities = vim.deepcopy(result.capabilities) end
  config.on_error = function(code, message) asynchronous_error = tostring(code) .. ": " .. tostring(message) end
  config.on_exit = function(code, signal) exited = { code = code, signal = signal } end
  local default_diagnostics = vim.lsp.handlers["textDocument/publishDiagnostics"]
  config.handlers = { ["textDocument/publishDiagnostics"] = function(err, result, context, handler_config)
    if err then asynchronous_error = vim.inspect(err); return end
    report.publications[#report.publications + 1] = vim.deepcopy(result)
    default_diagnostics(err, result, context, handler_config)
  end }
  buffer = api.nvim_create_buf(true, false)
  api.nvim_set_current_buf(buffer)
  local path = config.root_dir .. "/skript-lsp-semantic-" .. uv.os_getpid() .. "-" .. uv.hrtime() .. ".sk"
  assert(not uv.fs_stat(path))
  api.nvim_buf_set_name(buffer, path)
  vim.bo[buffer].filetype, vim.bo[buffer].swapfile = "skript", false
  uri = vim.uri_from_bufnr(buffer)
  lines = { "on load:", '    send "日本😀" to console', "    send sin(1) to console", "    send {_money} to console", "    # no semantic color here" }
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
  client_id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
  client = assert(vim.lsp.get_client_by_id(client_id))
  wait_for("semantic tokens", function() return token(2, "sin", "function") and token(3, "{_money}", "variable") end)
  local legend = report.capabilities.semanticTokensProvider.legend
  report.legend, report.standardOnly = legend, standard_only
  report.executable = config.cmd[1]
  assert(client.offset_encoding == "utf-16")
  if standard_only then
    assert(vim.deep_equal(legend.tokenTypes, { "function", "variable" }), vim.inspect(legend))
    assert(#at(1, "send") == 0 and #at(1, '"日本😀"') == 0)
    check("standard-client-omits-unsupported-custom-types")
  else
    wait_for("custom classifications", function() return token(1, "send", "skriptEffect") and token(1, '"日本😀"', "skriptLiteral") end)
    local mark = wait_for("standard highlight extmark", function() return inspect_mark(1, "send", "skriptEffect") end)
    assert(mark.highlight.fg == tonumber("58C4DD", 16), vim.inspect(mark))
    check("effect-highlight-extmark-and-color", mark)
    local palette = {}
    -- These expected defaults preserve the REPL's existing source-syntax roles.
    for role, rgb in pairs({
      skriptEvent = "C55F73", skriptCondition = "FC6255", skriptEffect = "58C4DD",
      skriptExpression = "83C167", skriptSection = "5CD0B3", skriptStructure = "9A72AC",
      variable = "00FFFF", ["function"] = "A0A0A0",
      skriptInterpolationDelimiter = "B0B0B0",
    }) do
      local highlight = api.nvim_get_hl(0, { name = "@lsp.type." .. role .. ".skript", link = true })
      assert(not highlight.link and highlight.fg == tonumber(rgb, 16), vim.inspect(highlight))
      palette[role] = highlight
    end
    local literal_group = api.nvim_get_hl(0, { name = "@lsp.type.skriptLiteral.skript", link = true })
    assert(literal_group.link == "Normal", vim.inspect(literal_group))
    check("repl-palette-and-theme-aware-literal", { palette = palette, literal = literal_group })
    local event_mark = wait_for("event highlight", function() return inspect_mark(0, "load", "skriptEvent") end)
    assert(event_mark.highlight.fg == tonumber("C55F73", 16), vim.inspect(event_mark))
    local literal = token(1, '"日本😀"', "skriptLiteral")
    assert(literal.start_col == 9 and literal.end_col == 21, vim.inspect(literal))
    check("unicode-token-decoded-to-exact-utf8", vim.deepcopy(literal))
    change({ "on load:", '    send "value: %sin(1)%" to console', '    send {_key%sin(1)%} to console' }, false)
    wait_for("interpolation children", function() return token(1, "sin", "function") and token(2, "sin", "function") end)
    local string_child, variable_child = at(1, "sin"), at(2, "sin")
    check("string-and-variable-interpolation-own-child-ranges", { string = string_child, variable = variable_child })
    assert(#string_child == 1 and #variable_child == 1, "parent color overlaps an interpolation child")
    local delimiters = {}
    for row = 1, 2 do
      for column = 0, #lines[row + 1] - 1 do
        if lines[row + 1]:sub(column + 1, column + 1) == "%" then
          local found = false
          for _, value in ipairs(vim.lsp.semantic_tokens.get_at_pos(buffer, row, column) or {}) do
            if value.start_col == column and value.end_col == column + 1 and value.type == "skriptInterpolationDelimiter" then found = true; delimiters[#delimiters + 1] = value end
          end
          assert(found, "interpolation delimiter did not retain its own range")
        end
      end
    end
    assert(#delimiters == 4)
    check("string-and-variable-percent-delimiters-match-repl-gray", delimiters)
    local interpolation_publication = change({ "on load:", '    send "100%%, %sin(1)%" to console', '    send {_key% sin(1) %} to console', '    send {_outer%{_inner%sin(1)%}%} to console' }, true)
    assert(vim.iter(interpolation_publication.diagnostics):any(function(diagnostic)
      return diagnostic.code == "parser.syntax-incomplete" and diagnostic.range.start.line == 2
    end), "the current parser rejects whitespace around an embedded expression")
    wait_for("nested interpolations", function() return token(3, "sin", "function") end)
    local counts = {}
    for row = 1, 3 do
      local count = 0
      for column = 0, #lines[row + 1] - 1 do
        for _, value in ipairs(vim.lsp.semantic_tokens.get_at_pos(buffer, row, column) or {}) do
          if value.start_col == column and value.end_col == column + 1 and value.type == "skriptInterpolationDelimiter" then count = count + 1 end
        end
      end
      counts[#counts + 1] = count
    end
    assert(vim.deep_equal(counts, { 2, 0, 4 }), vim.inspect(counts))
    local escaped = at(1, "%%")
    assert(#escaped == 1 and escaped[1].type == "skriptLiteral", vim.inspect(escaped))
    assert(#at(2, "%") == 0, "unparsed interpolation has no fabricated token")
    check("escaped-percent-nested-delimiters-and-unparsed-whitespace", counts)
    change({ "on join:", "    send name of player to console" }, false)
    wait_for("registered expression fixed words", function() return token(1, "name", "skriptExpression") and token(1, "of", "skriptExpression") end)
    check("expression-fixed-words-have-their-own-classification")
    change({ "on load:", "    if 1 is 1:", '        send "ok" to console' }, false)
    wait_for("condition child fixed words", function() return token(1, "is", "skriptCondition") and token(2, "send", "skriptEffect") end)
    wait_for("section fixed word", function() return token(1, "if", "skriptSection") end)
    assert(#at(1, "if") == 1 and #at(1, "is") == 1)
    local section_mark = wait_for("section highlight", function() return inspect_mark(1, "if", "skriptSection") end)
    assert(section_mark.highlight.fg == tonumber("5CD0B3", 16), vim.inspect(section_mark))
    check("native-regex-capture-range-separates-section-and-condition", section_mark)
    local condition_mark = wait_for("condition highlight", function() return inspect_mark(1, "is", "skriptCondition") end)
    assert(condition_mark.highlight.fg == tonumber("FC6255", 16), vim.inspect(condition_mark))
    assert(condition_mark.highlight.fg ~= event_mark.highlight.fg)
    check("event-and-condition-have-distinct-repl-colors", { event = event_mark, condition = condition_mark })
    change({ "function greet(message: string) :: string:", '    send "hello" to console', "on load:", '    send greet("hello") to console' }, false)
    wait_for("function Structure fixed word", function() return token(0, "function", "skriptStructure") and token(3, "greet", "function") end)
    local structure_mark = wait_for("Structure highlight", function() return inspect_mark(0, "function", "skriptStructure") end)
    assert(structure_mark.highlight.fg == tonumber("9A72AC", 16), vim.inspect(structure_mark))
    assert(#at(0, "greet") == 0 and #at(0, "message") == 0 and #at(0, "string") == 0,
      "unclassified declaration capture must not inherit the Structure prefix color")
    check("structure-fixed-prefix-keeps-repl-color-without-coloring-its-regex-capture", structure_mark)
    -- Reload the sample with one pre-existing override, without changing any file.
    api.nvim_set_hl(0, "@lsp.type.skriptCondition.skript", { fg = "#123456" })
    dofile(configuration)
    vim.lsp.enable("skript_lsp", false)
    assert(api.nvim_get_hl(0, { name = "@lsp.type.skriptCondition.skript", link = false }).fg == tonumber("123456", 16))
    assert(api.nvim_get_hl(0, { name = "@lsp.type.skriptLiteral.skript", link = true }).link == "Normal")
    check("sample-reload-preserves-user-highlight-override")
    api.nvim_set_hl(0, "@lsp.type.skriptCondition.skript", { fg = "#FC6255" })
  end
  change({ "on load:", "    send sin(1) to console", "    this effect does not exist" }, true)
  wait_for("healthy partial sibling", function() return token(1, "sin", "function") end)
  assert(#at(2, "this") == 0)
  check("partial-document-preserves-healthy-sibling")
  change({ "on load:", "    this effect does not exist", "    # no semantic color here" }, true)
  wait_for("old tokens removed", function() return #(vim.lsp.semantic_tokens.get_at_pos(buffer, 1, 9) or {}) == 0 end)
  assert(#at(2, "semantic") == 0)
  check("edit-clears-obsolete-function-and-skips-comments")
  change({ "on load:", "    send sin(1) to console" }, false)
  wait_for("healthy tokens restored", function() return token(1, "sin", "function") end)
  check("valid-edit-restores-tokens")
  api.nvim_buf_delete(buffer, { force = true })
  buffer = api.nvim_create_buf(true, false)
  api.nvim_set_current_buf(buffer)
  api.nvim_buf_set_name(buffer, path)
  vim.bo[buffer].filetype, vim.bo[buffer].swapfile = "skript", false
  lines = { "on load:", "    send {_fresh} to console" }
  api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
  assert(vim.lsp.buf_attach_client(buffer, client_id))
  wait_for("same URI reopened", function() return token(1, "{_fresh}", "variable") end)
  assert(#at(1, "send") == (standard_only and 0 or 1))
  check("same-uri-reopen-has-fresh-tokens")
  assert(not uv.fs_stat(path), "test wrote its memory document")
  phase = "shutdown"
  client:stop()
  wait_for("normal server exit", function() return exited end)
  assert(exited.code == 0 and exited.signal == 0, vim.inspect(exited))
  report.status, report.exit = "passed", exited
end, debug.traceback)
if not ok then
  report.error = failure
  if client then client:stop(true) end
end
report.elapsedMs = (uv.hrtime() - started) / 1e6
if output then
  vim.fn.mkdir(output, "p")
  vim.fn.writefile({ vim.json.encode(report) }, output .. "/result.json")
end
if not ok then io.stderr:write(failure .. "\n"); vim.cmd("cquit 1") else vim.cmd("qa!") end
