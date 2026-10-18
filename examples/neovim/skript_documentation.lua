-- Lexical documentation colors; code blocks do not claim successful semantic analysis.
local M = {}
local namespace = vim.api.nvim_create_namespace("skript.documentation")

local function next_character(text, offset)
  local byte = text:byte(offset)
  return offset + (byte < 0x80 and 1 or byte < 0xe0 and 2 or byte < 0xf0 and 3 or 4)
end

local function pattern(text, add)
  add(1, #text + 1, "SkriptDocPatternLiteral")
  local offset, groups = 1, {}
  while offset <= #text do
    local character = text:sub(offset, offset)
    if character == "\\" then
      offset = offset < #text and next_character(text, offset + 1) or offset + 1
    elseif character == "%" or character == "<" then
      local close = text:find(character == "%" and "%" or ">", offset + 1, true)
      if close then
        add(offset, close + 1, character == "%" and "SkriptDocType" or "SkriptDocRegex")
        add(offset, offset + 1, "SkriptDocDelimiter")
        add(close, close + 1, "SkriptDocDelimiter")
        offset = close + 1
      else
        offset = offset + 1
      end
    elseif character == "(" or character == "[" then
      groups[#groups + 1] = character
      add(offset, offset + 1, "SkriptDocDelimiter")
      offset = offset + 1
    elseif character == ")" or character == "]" then
      if groups[#groups] == (character == ")" and "(" or "[") then
        table.remove(groups)
        add(offset, offset + 1, "SkriptDocDelimiter")
      end
      offset = offset + 1
    elseif character == "|" or character == ":" or text:sub(offset, offset + 1) == "¦" then
      local finish = next_character(text, offset)
      add(offset, finish, "SkriptDocDelimiter")
      offset = finish
    else
      offset = next_character(text, offset)
    end
  end
end

local function code(text, add)
  -- These are Skript's quote/percent/variable frames, not a keyword dictionary.
  local frames = { { kind = "code" } }
  local offset = 1
  while offset <= #text do
    local frame = frames[#frames]
    local character = text:sub(offset, offset)
    local finish = next_character(text, offset)
    local doubled = text:sub(offset + 1, offset + 1) == character
    local role = frame.kind == "string" and "SkriptDocString"
      or frame.kind == "variable" and "SkriptDocVariable" or nil
    if frame.kind == "string" then
      if (character == '"' or character == "%") and doubled then
        finish = offset + 2
      elseif character == '"' then
        table.remove(frames)
      elseif character == "%" then
        role = "SkriptDocInterpolationDelimiter"
        frames[#frames + 1] = { kind = "code", interpolation = true }
      end
    elseif frame.kind == "variable" then
      if character == "{" then
        frames[#frames + 1] = { kind = "variable" }
      elseif character == "}" then
        table.remove(frames)
      elseif character == "%" and doubled then
        finish = offset + 2
      elseif character == "%" then
        role = "SkriptDocInterpolationDelimiter"
        frames[#frames + 1] = { kind = "code", interpolation = true }
      end
    elseif character == "%" and frame.interpolation then
      role = "SkriptDocInterpolationDelimiter"
      table.remove(frames)
    elseif character == '"' then
      role = "SkriptDocString"
      frames[#frames + 1] = { kind = "string" }
    elseif character == "{" then
      role = "SkriptDocVariable"
      frames[#frames + 1] = { kind = "variable" }
    elseif character == "#" and not frame.interpolation then
      add(offset, #text + 1, "SkriptDocComment")
      return
    elseif character:match("%d") and not text:sub(offset - 1, offset - 1):match("[%w_]") then
      local number = text:sub(offset):match("^%d+%.?%d*")
      finish, role = offset + #number, "SkriptDocNumber"
    elseif character:match("[%a_]") then
      local name = text:sub(offset):match("^[%w_]+")
      finish = offset + #name
      if text:sub(finish, finish) == "(" then role = "SkriptDocFunction" end
    elseif character:match("[%[%](),:]") then
      role = "SkriptDocDelimiter"
    end
    if role then add(offset, finish, role) end
    offset = finish
  end
end

-- Positions are UTF-8 bytes, matching extmarks; every token stays on one line.
function M.tokens(lines)
  local tokens, fence, language = {}, nil, nil
  for row, text in ipairs(lines) do
    local delimiter, info = text:match("^%s*([`~]+)%s*([^%s]*)%s*$")
    if delimiter and #delimiter >= 3 and not delimiter:find("[^" .. delimiter:sub(1, 1) .. "]") then
      if not fence then
        fence, language = delimiter, info
      elseif delimiter:sub(1, 1) == fence:sub(1, 1) and #delimiter >= #fence and info == "" then
        fence, language = nil, nil
      end
    elseif fence and (language == "skript" or language == "skript-pattern") then
      local function add(start, finish, role)
        if start == finish then return end
        local last = tokens[#tokens]
        if last and last.row == row - 1 and last.finish == start - 1 and last.role == role then
          last.finish = finish - 1
        else
          tokens[#tokens + 1] = { row = row - 1, start = start - 1, finish = finish - 1, role = role }
        end
      end
      if language == "skript-pattern" then pattern(text, add) else code(text, add) end
    end
  end
  return tokens
end

function M.highlight(buffer)
  if not vim.api.nvim_buf_is_valid(buffer) then return end
  vim.api.nvim_buf_clear_namespace(buffer, namespace, 0, -1)
  local lines = vim.api.nvim_buf_get_lines(buffer, 0, -1, false)
  for _, token in ipairs(M.tokens(lines)) do
    vim.api.nvim_buf_set_extmark(buffer, namespace, token.row, token.start, {
      end_col = token.finish, hl_group = token.role,
      priority = token.role == "SkriptDocPatternLiteral" and 125 or 150,
    })
  end
end

function M.setup()
  local function colors()
    for role, link in pairs({
      PatternLiteral = "Keyword", Type = "Type", Regex = "String", Delimiter = "Delimiter",
      String = "String", Variable = "@lsp.type.variable.skript", Number = "Number",
      Function = "@lsp.type.function.skript", Comment = "Comment",
      InterpolationDelimiter = "@lsp.type.skriptInterpolationDelimiter.skript",
    }) do
      vim.api.nvim_set_hl(0, "SkriptDoc" .. role, { link = link, default = true })
    end
  end
  colors()
  local group = vim.api.nvim_create_augroup("SkriptDocumentation", { clear = true })
  vim.api.nvim_create_autocmd("ColorScheme", { group = group, callback = colors })
  vim.api.nvim_create_autocmd("FileType", {
    group = group, pattern = { "markdown", "blink-cmp-documentation" },
    callback = function(event)
      local buffer = event.buf
      if vim.bo[buffer].buftype ~= "nofile" then return end
      if not vim.b[buffer].skript_documentation_attached then
        vim.b[buffer].skript_documentation_attached = true
        local pending = false
        vim.api.nvim_buf_attach(buffer, false, { on_lines = function()
          if pending then return end
          pending = true
          vim.schedule(function() pending = false; M.highlight(buffer) end)
        end })
      end
      M.highlight(buffer)
    end,
  })
end

return M
