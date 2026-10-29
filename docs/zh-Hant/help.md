<!-- help: help helphelp -->

# 查閲幫助

`:tutor` 打開可編輯的中文教程。`:help` 瀏覽完整手冊。

| 查詢 | 目的地 |
| --- | --- |
| `:help :write` | 命令説明；`:help :w` 也可用 |
| `:help 'editor.scrolloff'` | 配置選項及示例 |
| `:help 'scrolloff'` | 無歧義時可省略配置節名 |
| `:help w` | Normal 模式按鍵 |
| `:help i_<Esc>` | Insert 模式按鍵 |
| `:help v_;` | Select 模式按鍵 |
| `:help c_<Backspace>` | 命令行編輯按鍵 |

按鍵區分大小寫。`<C-o>` 等同於 `C-o`；`<Space>` 表示空格前綴。

`gL` / `gd` 跟隨文檔鏈接；`C-o` 返回，`C-i` 前進。
默認鍵位參考 Helix，Vim 鍵位請查閲 [Vim preset](manual/12-vims-hands-keys-preset-vim.md)。

切換界面與幫助語言：`:language zhs`、`:language zh`、`:language en`。
指定幫助語言：`:help :write@en`、`:help help@zh-Hans`、`:help help@zh-Hant`。
缺少譯文時會標明並顯示英文原文。

[目录 / 目錄](SUMMARY.md) · [English help](../en/help.html)
