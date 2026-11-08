-- yumete --import lazyvim: LazyVim alone, with the reader's extras
-- (lazyvim.json) but none of their own files, so that what LazyVim sets
-- can be told from what they set. Run with -u, in the import's own copy of
-- their config and data; nothing is installed and no lock file of theirs
-- is written.
local data = vim.fn.stdpath("data")
vim.opt.rtp:prepend(data .. "/lazy/lazy.nvim")
-- LazyVim loads lazyvim.config.<name> and then the reader's config.<name>;
-- these stand in for the reader's three.
for _, name in ipairs({ "config.options", "config.keymaps", "config.autocmds" }) do
  package.preload[name] = function() return {} end
end
require("lazy").setup({
  spec = { { "LazyVim/LazyVim", import = "lazyvim.plugins" } },
  root = data .. "/lazy",
  lockfile = os.getenv("YUMETE_LOCK"),
  install = { missing = false },
  checker = { enabled = false },
  change_detection = { enabled = false },
  rocks = { enabled = false },
})
