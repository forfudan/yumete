-- yumete --import nvim: what this Neovim has once it has started, written
-- as JSON to $YUMETE_DUMP. It reads and writes nothing else, and it never
-- requires a module: a require would load a plugin.
local out_path = os.getenv("YUMETE_DUMP")

local function write(t)
  local f = assert(io.open(out_path, "w"))
  f:write(vim.json.encode(t))
  f:close()
end

local function version()
  local v = vim.version()
  return string.format("%d.%d.%d", v.major, v.minor, v.patch)
end

-- Functions a map may call that have a name yumete knows: only from what
-- is loaded already.
local KNOWN = { "vim.lsp.buf", "vim.diagnostic", "telescope.builtin", "fzf-lua", "gitsigns", "oil" }
local function names_of_functions()
  local names = {}
  for _, mod in ipairs(KNOWN) do
    local m
    if mod:match("^vim%.") then
      m = vim.tbl_get(vim, unpack(vim.split(mod:sub(5), ".", { plain = true })))
    else
      m = package.loaded[mod]
    end
    if type(m) == "table" then
      for k, v in pairs(m) do
        if type(v) == "function" then names[v] = mod .. "." .. k end
      end
    end
  end
  return names
end

-- lazy.nvim's keys of a plugin not loaded yet are stubs that call a
-- function; the right side the spec gave is in lazy's handler. Keyed by
-- the raw keys: maparg() writes a space leader <Space>, nvim_get_keymap() " ".
local function lazy_stubs()
  local stubs = {}
  local lazy = package.loaded["lazy.core.config"]
  if type(lazy) ~= "table" or type(lazy.plugins) ~= "table" then return stubs end
  for _, plugin in pairs(lazy.plugins) do
    local keys = type(plugin._) == "table" and plugin._.handlers and plugin._.handlers.keys
    if type(keys) == "table" then
      for _, k in pairs(keys) do
        local rhs = k.rhs or k[2]
        if type(rhs) == "string" and type(k.lhs) == "string" then
          local modes = type(k.mode) == "table" and k.mode or { k.mode or "n" }
          for _, mode in ipairs(modes) do
            local ok, m = pcall(vim.fn.maparg, k.lhs, mode, false, true)
            if ok and type(m) == "table" and m.lhsraw then stubs[mode .. "\0" .. m.lhsraw] = rhs end
          end
        end
      end
    end
  end
  return stubs
end

local function dump()
  local out = { nvim = version(), leader = vim.g.mapleader, colorscheme = vim.g.colors_name,
    options = {}, keymaps = {}, autocmds = {}, loaded = {} }
  if vim.fn.has("nvim-0.9") == 0 then
    out.error = "too old"
    return write(out)
  end
  local fns = names_of_functions()
  local stubs = lazy_stubs()
  local scripts = {}
  for _, s in ipairs(vim.fn.getscriptinfo()) do scripts[s.sid] = s.name end
  for name, _ in pairs(vim.api.nvim_get_all_options_info()) do
    local ok, val = pcall(vim.api.nvim_get_option_value, name, {})
    if ok then
      local w = vim.api.nvim_get_option_info2(name, {})
      out.options[name] = { value = val, script = scripts[w.last_set_sid], line = w.last_set_linenr }
    end
  end
  local seen = {}
  for _, query in ipairs({ "n", "x", "s", "o", "i", "c", "t" }) do
    for _, m in ipairs(vim.api.nvim_get_keymap(query)) do
      local id = m.mode .. "\0" .. m.lhs
      if not seen[id] then
        seen[id] = true
        local rhs, callback, expr = m.rhs, nil, m.expr == 1
        if m.callback then
          local stub = m.lhsraw and stubs[(m.mode == " " and "n" or m.mode) .. "\0" .. m.lhsraw]
          if stub then
            rhs, expr = stub, false
          else
            callback = fns[m.callback] or true
          end
        end
        table.insert(out.keymaps, { mode = m.mode, lhs = m.lhs, rhs = rhs, desc = m.desc, callback = callback,
          expr = expr, script = scripts[m.sid], line = m.lnum })
      end
    end
  end
  for _, a in ipairs(vim.api.nvim_get_autocmds({ event = { "FileType", "TextYankPost" } })) do
    table.insert(out.autocmds, { event = a.event, pattern = a.pattern, desc = a.desc, group = a.group_name,
      command = a.command })
  end
  for _, mod in ipairs({ "ibl" }) do
    if package.loaded[mod] then table.insert(out.loaded, mod) end
  end
  write(out)
end

local function after_start()
  -- lazy.nvim fires User VeryLazy on UIEnter, and LazyVim lays its keymaps
  -- then; a headless Neovim has no UI, so this says it has one.
  pcall(vim.api.nvim_exec_autocmds, "UIEnter", { modeline = false })
  vim.wait(3000, function() return package.loaded["lazy"] == nil or vim.g.did_very_lazy == true end, 10)
  vim.schedule(function()
    local ok, err = pcall(dump)
    if not ok then io.stderr:write("yumete dump: " .. tostring(err) .. "\n") end
    vim.cmd("qall!")
  end)
end

-- Older than 0.9: said at once, with what every Neovim has (review,
-- 2026-11-09: nvim_create_autocmd below is 0.7's, and an older one stopped
-- here with an error and waited out the import's 10 s).
if vim.fn.has("nvim-0.9") == 0 then
  local f = assert(io.open(out_path, "w"))
  f:write(vim.fn.json_encode({ nvim = vim.fn.matchstr(vim.fn.execute("version"), [[NVIM v\zs[0-9A-Za-z.+-]\+]]), error = "too old" }))
  f:close()
  vim.cmd("qall!")
  return
end

-- -c runs before VimEnter: wait for it, after the config's own handlers.
if vim.v.vim_did_enter == 1 then
  after_start()
else
  vim.api.nvim_create_autocmd("VimEnter", { once = true, nested = true, callback = function() vim.schedule(after_start) end })
end
