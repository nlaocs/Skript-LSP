-- Real standard Markdown hover, isolated from the user's config and files.
local api, uv = vim.api, vim.uv
local report = { cases = {}, publications = {}, requests = {}, limitations = {
  "Headless Neovim standard floating windows and screen cells; no desktop screenshot or third-party Markdown plugin.",
  "Worker races and fault exclusion are covered by controlled server tests, not this presentation test.",
} }
local client, buffer, output, exited, asynchronous_error
local started = uv.hrtime()
local function wait(label, predicate)
  assert(vim.wait(90000, function() return predicate() or asynchronous_error or exited end, 10), "timeout: " .. label)
  assert(not asynchronous_error and not exited, asynchronous_error or "unexpected server exit")
  return assert(predicate(), label)
end
local function float()
  for _, win in ipairs(api.nvim_list_wins()) do
    if api.nvim_win_get_config(win).relative ~= "" then return win end
  end
end
local function snapshot(win)
  local buf = api.nvim_win_get_buf(win)
  vim.cmd("redraw!")
  local screen = {}
  for row = 1, vim.o.lines do
    local line = {}
    for col = 1, vim.o.columns do line[#line + 1] = vim.fn.screenstring(row, col) end
    screen[#screen + 1] = table.concat(line)
  end
  return { lines = api.nvim_buf_get_lines(buf, 0, -1, false),
    syntax = vim.bo[buf].syntax, filetype = vim.bo[buf].filetype,
    width = api.nvim_win_get_width(win), height = api.nvim_win_get_height(win),
    view = api.nvim_win_call(win, vim.fn.winsaveview), screen = screen }
end
local function formatting(win)
  local buf, result = api.nvim_win_get_buf(win), {}
  -- Parse injected inline markup outside the initial viewport as well.
  vim.treesitter.get_parser(buf):parse(true)
  for row, line in ipairs(api.nvim_buf_get_lines(buf, 0, -1, false)) do
    for _, expected in ipairs({ { "**Players:**", "markup.strong", "bold", 2 }, { "*custom*", "markup.italic", "italic", 1 } }) do
      local column = line:find(expected[1], 1, true)
      if column and not result[expected[2]] then
        local captures = vim.treesitter.get_captures_at_pos(buf, row - 1, column - 1 + expected[4])
        local matched = false
        for _, capture in ipairs(captures) do if capture.capture == expected[2] then matched = true end end
        local attributes = api.nvim_get_hl(0, { name = "@" .. expected[2], link = false })
        assert(matched and attributes[expected[3]], "standard Markdown emphasis missing: " .. expected[2] .. " " .. vim.inspect({ captures = captures, attributes = attributes }))
        result[expected[2]] = { row = row - 1, column = column - 1 + expected[4], captures = captures, attributes = attributes }
      end
    end
  end
  return result
end
local function contains(text, needle) assert(text:find(needle, 1, true), "missing " .. needle .. " in " .. text) end
local function count(text, needle)
  local n, position = 0, 1
  while true do local i = text:find(needle, position, true); if not i then return n end; n, position = n + 1, i + #needle end
end
local function hover(row, options)
  options = options or {}
  api.nvim_set_current_win(report.sourceWindow)
  api.nvim_win_set_cursor(0, { row + 1, options.column or 11 })
  local before = #report.requests
  if row == 1 and #report.cases == 0 then
    -- Exercise Neovim's default LSP K mapping, not a replacement callback.
    vim.cmd("normal K")
  else
    vim.lsp.buf.hover({ border = "single", max_width = 100, max_height = 25 })
  end
  local request = wait("standard hover", function() local r = report.requests[before + 1]; return r and r.completed and r end)
  assert(request.error == vim.NIL and request.result.contents.kind == "markdown", "Markdown response required")
  local win = wait("floating window", float)
  local observation = snapshot(win)
  observation.formatting = formatting(win)
  observation.response = request.result
  local start, ending = request.result.range.start, request.result.range["end"]
  assert(start.line == row and start.character == (options.start or 9), "wrong selected expression")
  observation.selectedSource = table.concat(api.nvim_buf_get_text(buffer, start.line,
    vim.str_byteindex(api.nvim_buf_get_lines(buffer, start.line, start.line + 1, false)[1], "utf-16", start.character, false),
    ending.line, vim.str_byteindex(api.nvim_buf_get_lines(buffer, ending.line, ending.line + 1, false)[1], "utf-16", ending.character, false), {}), "\n")
  assert(vim.w[win].lsp_floating_bufnr == buffer, "wrong floating source buffer")
  assert(observation.syntax == "markdown" or observation.filetype == "markdown", "standard Markdown renderer required")
  local text = table.concat(observation.lines, "\n")
  assert(not text:find("<strong>", 1, true) and not text:find("<em>", 1, true) and not text:find("&lt;", 1, true), "unconverted documentation in real float")
  if not options.literalProse then
    contains(text, "<red>")
    assert(count(text, "Examples") == 1, "Examples heading repeated in actual float")
  end
  report.cases[#report.cases + 1] = observation
  return observation, win
end
local function publish_after(before)
  return wait("current diagnostics", function() return report.publications[before + 1] end)
end
local function run()
  output = assert(vim.env.SKRIPT_LSP_SMOKE_OUTPUT)
  -- Include the installed, bundled parsers while excluding user plugins.
  local bundled = vim.fs.normalize(vim.env.VIMRUNTIME .. "/../../../lib/nvim")
  vim.opt.runtimepath, vim.opt.packpath = { vim.env.VIMRUNTIME, bundled }, {}
  assert(#api.nvim_get_runtime_file("parser/markdown.*", false) > 0, "bundled Markdown parser required")
  vim.o.modeline, vim.o.shadafile = false, "NONE"
  vim.o.columns, vim.o.lines = 140, 45
  vim.o.winborder = "single"
  vim.cmd("syntax enable")
  local root = assert(vim.env.SKRIPT_LSP_ROOT)
  local config = dofile(root .. "/examples/neovim/skript-lsp.lua")
  vim.lsp.enable("skript_lsp", false)
  config.name, config.detached, config.exit_timeout = "skript_lsp_hover_formatting", false, 2000
  config.flags = { allow_incremental_sync = false, debounce_text_changes = 0 }
  config.capabilities = vim.lsp.protocol.make_client_capabilities()
  config.capabilities.textDocument.hover.contentFormat = { "markdown", "plaintext" }
  config.on_error = function(code, message) asynchronous_error = tostring(code) .. ": " .. tostring(message) end
  config.on_exit = function(code, signal) exited = { code = code, signal = signal } end
  local default = vim.lsp.handlers["textDocument/publishDiagnostics"]
  config.handlers = { ["textDocument/publishDiagnostics"] = function(err, result, context, handler_config)
    if err then asynchronous_error = vim.inspect(err); return end
    report.publications[#report.publications + 1] = vim.deepcopy(result)
    default(err, result, context, handler_config)
  end }
  buffer = api.nvim_create_buf(true, false)
  api.nvim_set_current_buf(buffer)
  api.nvim_buf_set_name(buffer, config.root_dir .. "/hover-formatting-" .. uv.os_getpid() .. ".sk")
  vim.bo[buffer].swapfile, vim.bo[buffer].undofile, vim.bo[buffer].filetype = false, false, "skript"
  local source = { "on join:", "    send name of player to console", "    send display name of player to console", "    send player list name of player to console" }
  api.nvim_buf_set_lines(buffer, 0, -1, false, source)
  report.sourceWindow = api.nvim_get_current_win()
  local id = assert(vim.lsp.start(config, { bufnr = buffer, reuse_client = function() return false end }))
  client = assert(vim.lsp.get_client_by_id(id))
  local first = publish_after(0)
  assert(#first.diagnostics == 0, vim.inspect(first.diagnostics))
  local request = client.request
  client.request = function(self, method, params, handler, bufnr)
    if method ~= "textDocument/hover" then return request(self, method, params, handler, bufnr) end
    local event = { params = vim.deepcopy(params) }; report.requests[#report.requests + 1] = event
    return request(self, method, params, function(err, result, context, handler_config)
      event.error, event.result = err or vim.NIL, result or vim.NIL
      local ok, message = pcall(handler, err, result, context, handler_config)
      if not ok then asynchronous_error = tostring(message) end
      event.completed = true
    end, bufnr)
  end
  local classes = {
    "org.skriptlang.skript.common.properties.elements.expressions.PropExprName",
    "org.skriptlang.skript.common.properties.elements.expressions.PropExprCustomName",
    "org.skriptlang.skript.bukkit.entity.player.elements.expressions.ExprPlayerListName",
  }
  for row, class in ipairs(classes) do
    local observed, win = hover(row)
    observed.name = ({ "name-single-line-items", "display-name-multiline-item", "player-list-raw-color" })[row]
    if row ~= 3 then
      assert(observed.formatting["markup.strong"] and observed.formatting["markup.italic"], "required strong/italic documentation disappeared")
    end
    local text = table.concat(observed.lines, "\n")
    contains(text, class)
    if row == 1 then
      contains(text, "Players:")
      contains(text, "on join:\n\tplayer has permission \"name.red\"\n\tset the player's display name to \"<red>[admin] <gold>%name of player%\"")
      contains(text, "Legendary Sword of Awesomeness")
      assert(count(observed.response.contents.value, "```skript\n") == 2, "wrong example boundaries")
    else assert(count(observed.response.contents.value, "```skript\n") == 1, "multiline item split") end
    -- Invoke the same public hover action again: Neovim focuses its existing preview.
    vim.lsp.buf.hover({ border = "single", max_width = 100, max_height = 25 })
    wait("second hover focuses existing preview", function() return api.nvim_get_current_win() == win end)
    vim.cmd("normal! G")
    observed.scrolled = snapshot(win)
    assert(observed.scrolled.view.topline > 1, "long hover did not scroll")
    local screen = table.concat(observed.scrolled.screen, "\n")
    contains(screen, "<red>")
    assert(not screen:find("&lt;", 1, true) and not screen:find("<strong>", 1, true), "wrong actual screen cells")
    -- The preview's standard q mapping deletes its own preview buffer.
    vim.cmd("normal q")
    assert(not api.nvim_win_is_valid(win) and api.nvim_get_current_buf() == buffer, "preview close did not restore source")
    observed.operations = { focus = true, scroll = true, closeViaQ = true, openViaK = row == 1 }
    assert(vim.deep_equal(api.nvim_buf_get_lines(buffer, 0, -1, false), source), "hover operation changed source")
  end
  local before = #report.publications
  source[2] = "    send display name of player to console"
  api.nvim_buf_set_lines(buffer, 0, -1, false, source)
  local changed = publish_after(before)
  assert(changed.version > first.version and #changed.diagnostics == 0, "edit did not publish current healthy version")
  local edited, win = hover(1)
  edited.name = "edit-refreshes-documentation"
  assert(edited.formatting["markup.strong"] and edited.formatting["markup.italic"], "edited documentation lost emphasis")
  contains(table.concat(edited.lines, "\n"), classes[2])
  assert(not edited.response.contents.value:find(classes[1], 1, true), "stale Java class after edit")
  api.nvim_win_close(win, true)
  local before_angles = #report.publications
  source = { "on join:", '    log "angle probe"', '    send first 2 characters of "hello" to console', '    send stone named "hello" to console', "    send color of stone to console" }
  api.nvim_buf_set_lines(buffer, 0, -1, false, source)
  assert(#publish_after(before_angles).diagnostics == 0, "angle descriptions require a healthy real fixture")
  for _, query in ipairs({
    { name = "log-description-prose", row = 1, column = 5, start = 4, literal = "[<script>.sk]" },
    { name = "substring-description-prose", row = 2, column = 11, literal = "<x>" },
    { name = "named-description-code", row = 3, column = 16, literal = "<item/inventory>" },
    { name = "color-description-spaced-placeholder", row = 4, column = 11, literal = "<%color of ...%>" },
  }) do
    local observed, preview = hover(query.row, { column = query.column, start = query.start, literalProse = true })
    observed.name = query.name
    local value = observed.response.contents.value
    contains(value, query.literal)
    assert(not value:find("\\<", 1, true) and not value:find("\\>", 1, true), "server added angle escape slashes")
    local target
    for row, line in ipairs(observed.lines) do if line:find(query.literal, 1, true) then target = row; break end end
    assert(target, "literal description missing from actual buffer")
    api.nvim_set_current_win(preview)
    api.nvim_win_set_cursor(preview, { target, 0 })
    vim.cmd("normal! zz")
    observed.focusedDescription = snapshot(preview)
    api.nvim_set_current_win(report.sourceWindow)
    observed.unfocusedDescription = snapshot(preview)
    for _, view in ipairs({ observed.focusedDescription, observed.unfocusedDescription }) do
      local screen = table.concat(view.screen, "\n")
      contains(screen, query.literal)
      assert(not screen:find("\\<", 1, true) and not screen:find("\\>", 1, true), "escape slash visible in actual screen")
      assert(not screen:find("&lt;", 1, true) and not screen:find("&gt;", 1, true), "entity workaround exposed source markup")
    end
    api.nvim_win_close(preview, true)
  end
  -- Controlled renderer case: the real snapshot has no intentional prose path.
  -- Rust tests verify HTML -> this Markdown; here the actual renderer must keep
  -- the intended literal backslashes in focused and unfocused screen cells.
  local literal = [[C:\Skript\scripts\*.sk \\server\share \n &lt;x&gt; 日本😀]]
  local markdown = { [[`C:\Skript\scripts\*.sk` `\\server\share` `\n` `&lt;x&gt;` 日本😀]], [[**`<player>`** *`C:\path`* [`<item type>`](<https://example.test/>)]] }
  local control_buf, control_win = vim.lsp.util.open_floating_preview(markdown, "markdown", { border = "single" })
  vim.treesitter.get_parser(control_buf):parse(true)
  local styled = {}
  for _, check in ipairs({ { "<player>", "markup.strong", "bold" }, { [[C:\path]], "markup.italic", "italic" } }) do
    local column = assert(markdown[2]:find(check[1], 1, true))
    local captures = vim.treesitter.get_captures_at_pos(control_buf, 1, column)
    local found = false
    for _, capture in ipairs(captures) do if capture.capture == check[2] then found = true end end
    local attributes = api.nvim_get_hl(0, { name = "@" .. check[2], link = false })
    assert(found and attributes[check[3]], "styled literal lost Markdown emphasis")
    styled[check[2]] = { captures = captures, attributes = attributes }
  end
  api.nvim_set_current_win(control_win)
  local focused = snapshot(control_win)
  api.nvim_set_current_win(report.sourceWindow)
  local unfocused = snapshot(control_win)
  report.literalControl = { origin = "controlled standard renderer; not an SSG response", input = markdown, expected = literal, focused = focused, unfocused = unfocused, styled = styled }
  for _, view in ipairs({ focused, unfocused }) do
    local screen = table.concat(view.screen, "\n")
    for _, word in ipairs({ [[C:\Skript\scripts\*.sk]], [[\\server\share]], [[\n]], [[&lt;x&gt;]], "日本😀", "<player>", [[C:\path]], "<item type>" }) do contains(screen, word) end
  end
  api.nvim_win_close(control_win, true)
  assert(#report.cases == 8, "all existing and angle-prose cases must execute")
  client.request = request
  api.nvim_buf_delete(buffer, { force = true })
  client:stop(false)
  assert(vim.wait(5000, function() return exited ~= nil end, 10), "shutdown timed out")
  assert(exited.code == 0 and exited.signal == 0, "abnormal server exit")
end
local ok, message = xpcall(run, debug.traceback)
if client and not exited then client:stop(true); vim.wait(5000, function() return exited ~= nil end, 10) end
report.status, report.error, report.serverExit = ok and "passed" or "failed", ok and vim.NIL or tostring(message), exited or vim.NIL
report.elapsedMs = (uv.hrtime() - started) / 1e6
if output then local file = assert(io.open(output, "wb")); assert(file:write(vim.json.encode(report), "\n")); assert(file:close()) end
if not ok then io.stderr:write(tostring(message), "\n") end
vim.cmd(ok and "qall!" or "cquit 1")
